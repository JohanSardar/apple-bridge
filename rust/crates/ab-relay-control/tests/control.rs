#![allow(clippy::unwrap_used, clippy::expect_used)]
use ab_relay_control::{
    Accounts, Admitted, Control, Error, RelayAdmin, RelayFuture, Session, SessionFuture, router,
};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use tower::ServiceExt;

const PUBLIC: &str = "https://app.example.test";
const HOST: &str = "app.example.test";
static SOURCE: AtomicU32 = AtomicU32::new(1);

struct Fixture;
impl Accounts for Fixture {
    fn session<'a>(&'a self, cookie: &'a str) -> SessionFuture<'a> {
        Box::pin(async move {
            let now = jiff::Timestamp::now().as_second();
            match cookie {
                "ada" => Ok(Session {
                    account: "ada".into(),
                    email: "ada@example.test".into(),
                    signed_in_at: now,
                }),
                "bea" => Ok(Session {
                    account: "bea".into(),
                    email: "bea@example.test".into(),
                    signed_in_at: now,
                }),
                "stale" => Ok(Session {
                    account: "ada".into(),
                    email: "ada@example.test".into(),
                    signed_in_at: now - 7200,
                }),
                _ => Err(Error::Unauthorized),
            }
        })
    }
}
#[derive(Default)]
struct Relay {
    admitted: Mutex<Vec<String>>,
    policy: Mutex<Vec<(String, bool, bool)>>,
    offline: std::sync::atomic::AtomicBool,
}
impl RelayAdmin for Relay {
    fn admit<'a>(&'a self, public_key: &'a str) -> RelayFuture<'a, Admitted> {
        Box::pin(async move {
            let mut admitted = self.admitted.lock().unwrap();
            admitted.push(public_key.to_owned());
            Ok(Admitted {
                hostname: format!("{:032x}.ab.example.test", admitted.len()),
                credential: "relay-credential".into(),
            })
        })
    }
    fn policy<'a>(
        &'a self,
        hostname: &'a str,
        disabled: bool,
        revoked: bool,
    ) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            if self.offline.load(Ordering::SeqCst) {
                return Err(Error::Unavailable);
            }
            self.policy
                .lock()
                .unwrap()
                .push((hostname.to_owned(), disabled, revoked));
            Ok(())
        })
    }
}
struct World {
    _dir: tempfile::TempDir,
    control: Arc<Control>,
    relay: Arc<Relay>,
    app: axum::Router,
}
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let control = Arc::new(
        Control::open(
            &dir.path().join("control.sqlite"),
            &dir.path().join("epoch"),
            PUBLIC,
        )
        .unwrap(),
    );
    let relay = Arc::new(Relay::default());
    let app = router(
        control.clone(),
        Arc::new(Fixture),
        relay.clone(),
        "http://127.0.0.1:1",
    )
    .unwrap();
    World {
        _dir: dir,
        control,
        relay,
        app,
    }
}
impl World {
    async fn send(&self, request: Request<Body>) -> (StatusCode, Value, axum::http::HeaderMap) {
        let source: SocketAddr = format!(
            "198.51.100.{}:1",
            SOURCE.fetch_add(1, Ordering::SeqCst) % 250 + 1
        )
        .parse()
        .unwrap();
        let mut request = request;
        request.extensions_mut().insert(ConnectInfo(source));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            headers,
        )
    }
    async fn post(&self, action: &str, cookie: Option<&str>, body: &Value) -> (StatusCode, Value) {
        let mut request = Request::post(format!("/v1/{action}"))
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/json");
        if !action.starts_with("device-") {
            request = request.header(header::ORIGIN, PUBLIC);
        }
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let (status, value, _) = self
            .send(request.body(Body::from(body.to_string())).unwrap())
            .await;
        (status, value)
    }
    async fn link(&self, cookie: &str, seed: u8) -> Value {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let public = B64.encode(key.verifying_key().as_bytes());
        let (status, start) = self
            .post(
                "device-start",
                None,
                &json!({"public_key": public, "name": "Ada's Mac"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let code = start["user_code"].as_str().unwrap();
        let (status, view) = self
            .post("grant-view", Some(cookie), &json!({"code": code}))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["fingerprint"], start["fingerprint"]);
        assert_eq!(view["name"], "Ada's Mac");
        let (status, confirmed) = self
            .post(
                "confirm",
                Some(cookie),
                &json!({"code": code, "fingerprint": start["fingerprint"]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        let poll = start["polling_secret"].as_str().unwrap();
        let proof = self.approved_poll(poll);
        let message = ab_wire::framing::device_link_message(
            PUBLIC,
            proof["transaction"].as_str().unwrap(),
            proof["nonce"].as_str().unwrap(),
            &public,
        );
        let (status, done) = self.post("device-complete", None, &json!({"polling_secret": poll, "signature": B64.encode(key.sign(&message).to_bytes())})).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        done
    }
    fn approved_poll(&self, poll: &str) -> Value {
        let now = jiff::Timestamp::now().as_second();
        self.control.poll(poll, now + 30).unwrap()
    }
}

#[tokio::test]
async fn a_person_links_their_mac_and_the_relay_admits_only_after_proof() {
    let w = world();
    let key = SigningKey::from_bytes(&[1; 32]);
    let public = B64.encode(key.verifying_key().as_bytes());
    let (_, start) = w
        .post("device-start", None, &json!({"public_key": public}))
        .await;
    let code = start["user_code"].as_str().unwrap();
    assert_eq!(code.len(), 9);
    assert_eq!(start["verification_uri"], format!("{PUBLIC}/device"));
    let poll = start["polling_secret"].as_str().unwrap();
    let wait = w.control.poll(poll, jiff::Timestamp::now().as_second() + 6);
    assert!(matches!(wait, Err(Error::Pending)), "not approved yet");
    assert!(w.relay.admitted.lock().unwrap().is_empty());

    let (status, _) = w.post("grant-view", None, &json!({"code": code})).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a person must be signed in"
    );

    let (status, _) = w
        .post(
            "confirm",
            Some("stale"),
            &json!({"code": code, "fingerprint": start["fingerprint"]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "linking needs a recent sign-in"
    );
    let (status, _) = w
        .post(
            "confirm",
            Some("ada"),
            &json!({"code": code, "fingerprint": "AAAA-BBBB"}),
        )
        .await;
    assert_eq!(status, StatusCode::GONE, "the fingerprint must match");
    let (status, _) = w.post("confirm", Some("ada"), &json!({"code": code.to_lowercase().replace('-', ""), "fingerprint": start["fingerprint"]})).await;
    assert_eq!(status, StatusCode::OK, "codes are typed by people");

    let proof = w.approved_poll(poll);
    let forged = SigningKey::from_bytes(&[9; 32]);
    let message = ab_wire::framing::device_link_message(
        PUBLIC,
        proof["transaction"].as_str().unwrap(),
        proof["nonce"].as_str().unwrap(),
        &public,
    );
    let (status, _) = w.post("device-complete", None, &json!({"polling_secret": poll, "signature": B64.encode(forged.sign(&message).to_bytes())})).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "only the Mac's own key completes the link"
    );
    assert!(w.relay.admitted.lock().unwrap().is_empty());

    let (status, done) = w.post("device-complete", None, &json!({"polling_secret": poll, "signature": B64.encode(key.sign(&message).to_bytes())})).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(*w.relay.admitted.lock().unwrap(), vec![public]);
    assert_eq!(done["credential"], "relay-credential");
    let (status, _) = w.post("device-complete", None, &json!({"polling_secret": poll, "signature": B64.encode(key.sign(&message).to_bytes())})).await;
    assert_eq!(status, StatusCode::GONE, "single use");
}

#[tokio::test]
async fn an_account_holds_two_macs_and_sees_only_its_own() {
    let w = world();
    let first = w.link("ada", 1).await;
    w.link("ada", 2).await;
    let key = SigningKey::from_bytes(&[3; 32]);
    let (_, start) = w
        .post(
            "device-start",
            None,
            &json!({"public_key": B64.encode(key.verifying_key().as_bytes())}),
        )
        .await;
    let (status, _) = w
        .post(
            "confirm",
            Some("ada"),
            &json!({"code": start["user_code"], "fingerprint": start["fingerprint"]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "two Macs per account"
    );
    let (_, list) = w.post("list", Some("ada"), &json!({})).await;
    assert_eq!(list["devices"].as_array().unwrap().len(), 2);
    let (_, other) = w.post("list", Some("bea"), &json!({})).await;
    assert_eq!(other["devices"].as_array().unwrap().len(), 0);
    let (status, _) = w
        .post(
            "revoke",
            Some("bea"),
            &json!({"hostname": first["hostname"]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "no reach into another account"
    );
    assert!(w.relay.policy.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tunnel_changes_are_committed_first_and_reach_the_relay_even_if_it_was_down() {
    let w = world();
    let done = w.link("ada", 1).await;
    let hostname = done["hostname"].as_str().unwrap();
    let (status, _) = w
        .post("suspend", Some("ada"), &json!({"hostname": hostname}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        *w.relay.policy.lock().unwrap(),
        vec![(hostname.to_owned(), true, false)]
    );
    let (status, _) = w
        .post("resume", Some("ada"), &json!({"hostname": hostname}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = w
        .post("revoke", Some("stale"), &json!({"hostname": hostname}))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "removal needs a recent sign-in"
    );

    w.relay.offline.store(true, Ordering::SeqCst);
    let (status, _) = w
        .post("revoke", Some("ada"), &json!({"hostname": hostname}))
        .await;
    assert_eq!(status, StatusCode::OK, "committed even with the relay down");
    let (_, list) = w.post("list", Some("ada"), &json!({})).await;
    assert_eq!(list["devices"].as_array().unwrap().len(), 0);
    assert_eq!(
        w.control.pending_policy().unwrap().len(),
        1,
        "queued for the relay"
    );

    w.relay.offline.store(false, Ordering::SeqCst);
    let task = tokio::spawn(ab_relay_control::sync_policy(
        w.control.clone(),
        w.relay.clone(),
    ));
    for _ in 0..100 {
        if w.control.pending_policy().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(
        w.relay
            .policy
            .lock()
            .unwrap()
            .contains(&(hostname.to_owned(), true, true))
    );
    let (status, _) = w
        .post("resume", Some("ada"), &json!({"hostname": hostname}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "revoked for good");
}

#[tokio::test]
async fn pages_and_writes_are_held_to_one_host_and_origin() {
    let w = world();
    let (status, _, _) = w
        .send(
            Request::get("/")
                .header(header::HOST, "0123.ab.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "wrong host");
    let (status, _, _) = w
        .send(
            Request::post("/v1/list")
                .header(header::HOST, HOST)
                .header(header::ORIGIN, "https://0123.ab.example.test")
                .header(header::COOKIE, "ada")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a sibling tunnel host may not write"
    );
    let (status, _, headers) = w
        .send(
            Request::get("/")
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "signed out goes to sign-in");
    assert_eq!(headers[header::LOCATION], "/sign-in");
    let (status, _, headers) = w
        .send(
            Request::get("/sign-in")
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'self'") && !csp.contains("unsafe"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn an_account_can_manage_tunnels_and_nothing_else() {
    let w = world();
    for action in [
        "records.query",
        "connector",
        "approve",
        "enroll",
        "pair",
        "token",
    ] {
        let (status, _) = w.post(action, Some("ada"), &json!({})).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{action} does not exist here"
        );
    }
    for path in [
        "/mcp",
        "/oauth/authorize",
        "/approvals",
        "/v1/op/records.query",
    ] {
        let (status, _, _) = w
            .send(
                Request::get(path)
                    .header(header::HOST, HOST)
                    .header(header::COOKIE, "ada")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
            "{path} {status}"
        );
    }
}

#[tokio::test]
async fn a_restored_backup_cannot_bring_back_a_revoked_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("control.sqlite");
    let fence = dir.path().join("epoch");
    let control = Control::open(&database, &fence, PUBLIC).unwrap();
    control.throttle("x", 1).unwrap();
    drop(control);
    std::fs::copy(&database, dir.path().join("backup.sqlite")).unwrap();
    let control = Control::open(&database, &fence, PUBLIC).unwrap();
    control.revoke_all("ada").unwrap();
    drop(control);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.path().join(format!("control.sqlite{suffix}")));
    }
    std::fs::copy(dir.path().join("backup.sqlite"), &database).unwrap();
    assert!(matches!(
        Control::open(&database, &fence, PUBLIC),
        Err(Error::Fenced)
    ));
}
