use crate::{
    Estimate, Result, Session,
    actions::{common, read_json},
    failed,
};
use lab_relay::{Frame, Identity, Link, Principal, authenticate, client_config};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

const FRAME_WAIT: Duration = Duration::from_secs(10);

struct Route {
    proxy: String,
    host: String,
    port: u16,
}
fn route(ready: &str, address: &str) -> Result<Route> {
    let proxy = read_json(ready)?;
    let url = proxy["url"].as_str().unwrap_or_default();
    let valid = url
        .strip_prefix("http://")
        .filter(|rest| rest.starts_with("127.0.0.1:"))
        .map(str::to_owned);
    let Some(proxy) = valid else {
        return failed("invalid public route");
    };
    let rest = address
        .strip_prefix("https://")
        .unwrap_or_default()
        .trim_end_matches('/');
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (host.to_owned(), port.parse().unwrap_or(443)),
        None => (rest.to_owned(), 443),
    };
    if host.is_empty() {
        return failed("invalid public route");
    }
    Ok(Route { proxy, host, port })
}
fn dial(route: &Route, roots: &rustls::RootCertStore) -> Result<Link> {
    let config = client_config(roots.clone())?;
    Ok(Link::dial_via_proxy(
        &route.proxy,
        &route.host,
        route.port,
        &config,
        None,
    )?)
}
fn enrollment(home: &str) -> Result<Value> {
    let credentials = read_json(&format!("{home}/credentials.json"))?;
    let enrollment = credentials["enrollment"].clone();
    if enrollment.is_null() {
        return failed("enrollment missing");
    }
    Ok(enrollment)
}
fn pin(enrollment: &Value) -> String {
    enrollment["fingerprint"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub fn replay<R: Read, W: Write>(
    session: &mut Session<R, W>,
    roots: &rustls::RootCertStore,
) -> Result<()> {
    let input = common(&session.input::<Value>()?)?;
    let enrollment = enrollment(&input.home)?;
    let identity = Identity::load(Path::new(&input.home))?;
    let route = route(
        &input.proxy_ready,
        enrollment["address"].as_str().unwrap_or_default(),
    )?;
    let mut first = dial(&route, roots)?;
    let hello = lab_relay::hello(&identity, false, None);
    first.send_text(&hello.text()?)?;
    let challenge = Frame::parse(&first.recv_text(FRAME_WAIT)?)?;
    let (der, key) = lab_relay::gateway_key(&challenge)?;
    let gateway_signed = lab_relay::verify(
        &key,
        &challenge.signature_bytes()?,
        &lab_relay::input1("gateway", &hello, &challenge, None)?,
    );
    if challenge.t != "AuthChallenge"
        || lab_relay::fingerprint(&der) != pin(&enrollment)
        || !gateway_signed
    {
        return failed("gateway identity rejected");
    }
    let proof = lab_relay::proof_frame(
        false,
        &identity.sign(&lab_relay::input1("replica", &hello, &challenge, None)?),
        None,
    );
    first.send_text(&proof.text()?)?;
    let accepted = Frame::parse(&first.recv_text(FRAME_WAIT)?)?;
    let principal = accepted.principal_text().map(str::to_owned);
    let accepted_signed = lab_relay::verify(
        &key,
        &accepted.signature_bytes()?,
        &lab_relay::input1("accepted", &hello, &challenge, principal.as_deref())?,
    );
    if accepted.t != "Authenticated" || !accepted_signed {
        return failed("capture not authenticated");
    }
    drop(first);
    let mut fresh = dial(&route, roots)?;
    fresh.send_text(&hello.text()?)?;
    let next = Frame::parse(&fresh.recv_text(FRAME_WAIT)?)?;
    if next.nonce == challenge.nonce {
        return failed("nonce reused");
    }
    fresh.send_text(&proof.text()?)?;
    if fresh.try_recv_text(Duration::from_secs(1)).is_ok() {
        return failed("replay was not rejected");
    }
    session
        .emit(&json!({"capture_authenticated": true, "fresh_nonce": true, "replay_rejected": true}))
}

enum Closed {
    At(i128),
    Never,
}
fn pump(mut link: Link, closed: &mpsc::Sender<Closed>, stop: &std::sync::atomic::AtomicBool) {
    loop {
        match link.try_recv_text(Duration::from_millis(100)) {
            Ok(Some(text)) => {
                let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let batch = &frame["batch"];
                if !batch.is_null() {
                    let cursor = json!({"epoch": batch["epoch"], "subscription_revision": batch["subscription_revision"], "sequence": batch["through"]});
                    let ack = json!({"t": "Ack", "cursor": cursor, "credits": 4 * 1024 * 1024});
                    let _ = link.send_text(&ack.to_string());
                }
            }
            Ok(None) => {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = closed.send(Closed::Never);
                    return;
                }
            }
            Err(_) => {
                let at = jiff::Timestamp::now().as_nanosecond();
                let _ = closed.send(if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    Closed::Never
                } else {
                    Closed::At(at)
                });
                return;
            }
        }
    }
}

fn welcomed(link: &mut Link, hello: &Value) -> Result<()> {
    link.send_text(&hello.to_string())?;
    let welcome: Value = serde_json::from_str(&link.recv_text(FRAME_WAIT)?)?;
    if welcome["t"] != "Welcome" {
        return failed("revocation feed not welcomed");
    }
    Ok(())
}

pub fn revocation<R: Read, W: Write>(
    session: &mut Session<R, W>,
    roots: &rustls::RootCertStore,
) -> Result<()> {
    let input = common(&session.input::<Value>()?)?;
    let enrollment = enrollment(&input.home)?;
    let identity = Identity::load(Path::new(&input.home))?;
    let route = route(
        &input.proxy_ready,
        enrollment["address"].as_str().unwrap_or_default(),
    )?;
    let mut link = dial(&route, roots)?;
    let principal: Principal = authenticate(&mut link, &identity, false, &pin(&enrollment), None)?;
    if principal.id != enrollment["replica_id"].as_str().unwrap_or_default() {
        return failed("revocation connection unauthenticated");
    }
    let local = crate::Local::new(&input)?;
    let head = local.op("records.query", &json!({"collection": "", "limit": 1}))?;
    let hello = json!({"t": "Hello", "protocol": "feed/1", "replica_id": enrollment["replica_id"], "cursor": head["cursor"], "schema_offer": [1], "credits": 4 * 1024 * 1024});
    welcomed(&mut link, &hello)?;
    let (sender, receiver) = mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let worker = std::thread::spawn(move || pump(link, &sender, &flag));
    session.ready()?;
    session.signal()?;
    let clock = Estimate::load(Path::new(&input.home))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let outcome = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = worker.join();
    match outcome {
        Ok(Closed::At(at)) => session.emit(&json!({
            "closed_gateway_wall_ns": i64::try_from(at + clock.offset_ns).unwrap_or_default(),
            "clock_uncertainty_ms": clock.uncertainty_ms(),
        })),
        Ok(Closed::Never) => failed("revocation close unverified"),
        Err(_) => failed("context deadline exceeded"),
    }
}
