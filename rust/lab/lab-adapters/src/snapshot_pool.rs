use crate::{assertions::await_until, http::Client, replicas};
use lab_relay::{FeedLink, Identity};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    time::{Duration, Instant},
};

const BYTE_CREDIT: u64 = 4 * 1024 * 1024;
const FEEDS: usize = 12;

#[derive(Debug)]
pub struct Feed {
    pub link: FeedLink,
    pub replica_id: String,
    pub pin: String,
    pub address: String,
}

pub fn identity_of(index: usize) -> Result<(Identity, String, String, String)> {
    let endpoint = replicas::endpoints()
        .get(index)
        .cloned()
        .ok_or_else(|| Error::Invalid("replica endpoint".into()))?;
    let identity = Identity::load(&endpoint.home).map_err(|e| Error::Invalid(e.to_string()))?;
    let document: Value =
        serde_json::from_slice(&std::fs::read(endpoint.home.join("credentials.json"))?)?;
    let enrollment = &document["enrollment"];
    let text = |key: &str| enrollment[key].as_str().unwrap_or_default().to_owned();
    let (address, pin, replica) = (text("address"), text("fingerprint"), text("replica_id"));
    if address.is_empty() || replica.is_empty() {
        return Err(Error::Invalid("replica enrollment missing".into()));
    }
    Ok((identity, address, pin, replica))
}

pub fn dial(index: usize, timeout: Duration) -> Result<Feed> {
    let (identity, address, pin, replica_id) = identity_of(index)?;
    let link = FeedLink::dial(&address, &identity, &pin, timeout)
        .map_err(|e| Error::Invalid(format!("feed dial: {e}")))?;
    Ok(Feed {
        link,
        replica_id,
        pin,
        address,
    })
}

pub fn hello(feed: &mut Feed) -> Result<()> {
    let frame = json!({"t":"Hello","protocol":"feed/1","replica_id":feed.replica_id,"schema_offer":[1],"credits":BYTE_CREDIT});
    feed.link
        .send_text(&frame.to_string())
        .map_err(|e| Error::Invalid(e.to_string()))
}

pub fn ack(feed: &mut Feed, credits: usize, cursor: Option<&Value>) -> Result<()> {
    let mut frame = json!({"t":"Ack","credits":credits});
    if let Some(cursor) = cursor {
        frame["cursor"] = cursor.clone();
    }
    feed.link
        .send_text(&frame.to_string())
        .map_err(|e| Error::Invalid(e.to_string()))
}

pub fn read(feed: &mut Feed, deadline: Instant) -> Result<(Value, usize)> {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(50));
    let text = feed
        .link
        .recv_text(remaining)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let size = text.len();
    Ok((serde_json::from_str(&text)?, size))
}

pub fn digest(records: &[Value]) -> String {
    let mut ordered: Vec<&Value> = records.iter().collect();
    let key = |record: &Value| {
        format!(
            "{}/{}",
            record["collection"].as_str().unwrap_or_default(),
            record["id"].as_str().unwrap_or_default()
        )
    };
    ordered.sort_by_key(|record| key(record));
    let mut text = String::new();
    for record in ordered {
        let _ = writeln!(
            text,
            "{}\0{}\0{}",
            record["collection"].as_str().unwrap_or_default(),
            record["id"].as_str().unwrap_or_default(),
            record["version"].as_u64().unwrap_or_default()
        );
    }
    hex::encode(Sha256::digest(text.as_bytes()))
}

struct Paused {
    feed: Feed,
    bytes: usize,
    begin: Value,
    cursor: Value,
}

fn open_paused(index: usize, deadline: Instant, paused: &mut Vec<Paused>) -> Result<()> {
    let mut feed = dial(index, Duration::from_secs(5))?;
    hello(&mut feed)?;
    let mut cursor = Value::Null;
    loop {
        let (frame, size) = read(&mut feed, deadline)?;
        match frame["t"].as_str() {
            Some("Heartbeat") => continue,
            Some("Welcome") => {
                cursor = json!({"epoch":frame["epoch"],"subscription_revision":frame["subscription_revision"]});
            }
            Some("SnapshotBegin") => {
                cursor["sequence"] = frame["s"].clone();
                paused.push(Paused {
                    feed,
                    bytes: size,
                    begin: frame,
                    cursor,
                });
                return Ok(());
            }
            _ => {}
        }
        ack(&mut feed, size, None)?;
    }
}

fn resume(item: &mut Paused, deadline: Instant) -> Result<bool> {
    ack(&mut item.feed, item.bytes, None)?;
    let mut records: Vec<Value> = vec![];
    loop {
        let (frame, size) = read(&mut item.feed, deadline)?;
        if frame["t"] == "Heartbeat" {
            continue;
        }
        if frame["t"] == "SnapshotPage" {
            records.extend(frame["records"].as_array().cloned().unwrap_or_default());
        }
        let end = frame["t"] == "SnapshotEnd";
        ack(&mut item.feed, size, end.then_some(&item.cursor))?;
        if end {
            let count = usize::try_from(item.begin["count"].as_u64().unwrap_or(0)).unwrap_or(0);
            return Ok(records.len() == count
                && item.begin["digest"].as_str() == Some(digest(&records).as_str()));
        }
    }
}

fn unproven(client: &Client) -> bool {
    client
        .call("sources.status", &json!({}), 5)
        .ok()
        .flatten()
        .is_some_and(|sources| {
            sources["records"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|record| {
                    record["source"] == "messages"
                        && record["data"]["reason"] == "capability_unproven"
                })
        })
}

pub fn run(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let result = cases(world, input, evidence);
    crate::labcore::oversized_resets(world, evidence);
    result
}
fn cases(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    world.control("/seed", &json!({"count": 1}))?;
    world.control_for("/flush", &json!({}), 30)?;
    let paired = replicas::start_with(world, 1, "admin", 0).is_ok();
    evidence.check("paired_replica", paired, "");
    if !paired {
        return Ok(());
    }
    let admin = Client::new(world, "admin")?;
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut feeds: Vec<Paused> = vec![];
    let started = Instant::now();
    while feeds.len() < FEEDS {
        if let Err(error) = open_paused(0, deadline, &mut feeds) {
            evidence.check(
                "twelve_paused_snapshots_with_pool_8",
                false,
                &error.to_string(),
            );
            return Ok(());
        }
    }
    evidence.check(
        "twelve_paused_snapshots_with_pool_8",
        true,
        &format!("{} ms", started.elapsed().as_millis()),
    );
    let reads = admin
        .call("sources.status", &json!({}), 5)
        .ok()
        .flatten()
        .is_some_and(|sources| sources["records"].as_array().is_some_and(|r| !r.is_empty()));
    evidence.check("reads_progress_with_paused_snapshots", reads, "");
    evidence.check(
        "paused_source_change_queued",
        world
            .control("/messages", &json!({"action": "capability-unproven"}))
            .is_ok(),
        "",
    );
    evidence.check(
        "writes_progress_with_paused_snapshots",
        await_until(3, || unproven(&admin)),
        "",
    );
    for item in &mut feeds {
        if let Ok(frozen) = resume(item, deadline) {
            evidence.check("materialized_snapshot_stays_frozen", frozen, "");
        } else {
            evidence.check("paused_snapshot_resumes", false, "");
            return Ok(());
        }
    }
    evidence.check("paused_snapshot_resumes", true, "");
    Ok(())
}
