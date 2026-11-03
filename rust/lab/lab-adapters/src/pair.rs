use crate::{
    assertions::{Observed, await_until, converged},
    http::Client,
    journey::{GATEWAY_SCOPE, gateway_client},
    replicas,
    robust::automatic_healthy,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use jiff::{SignedDuration, Timestamp};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const ENCODING: data_encoding::Encoding = data_encoding::BASE32_NOPAD;

fn decode(code: &str) -> Result<Value> {
    let raw = ENCODING
        .decode(code.trim_start_matches("ab1-").as_bytes())
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(serde_json::from_slice(&raw)?)
}
fn encode(value: &Value) -> Result<String> {
    Ok(format!("ab1-{}", ENCODING.encode(&serde_json::to_vec(value)?)))
}
fn rejected(home: &Path, code: &str) -> bool {
    replicas::cli(home, &["join", code]).is_err()
}

fn pairing_checks(world: &World) -> Result<bool> {
    let home = world.directory.join("pairing");
    fs::create_dir_all(&home)?;
    let address = world.pair_address()?;
    let issue = |scope: &str, extra: &[&str]| -> Result<String> {
        let mut args = vec!["pair", "--peer-addr", &address, "--scope", scope];
        args.extend_from_slice(extra);
        Ok(world.gateway_cli(&args)?.trim().to_owned())
    };
    let code = issue("reminders:read,system:read", &[])?;
    let mut wrong = decode(&code)?;
    let mut secret = STANDARD
        .decode(wrong["secret"].as_str().unwrap_or_default())
        .map_err(|e| Error::Invalid(e.to_string()))?;
    if let Some(first) = secret.first_mut() {
        *first ^= 1;
    }
    wrong["secret"] = json!(STANDARD.encode(secret));
    if !rejected(&home, &encode(&wrong)?) {
        return Ok(false);
    }
    if replicas::cli(&home, &["join", &code]).is_err() || !rejected(&home, &code) {
        return Ok(false);
    }
    let expired = issue("reminders:read", &["--expires-in", "1ms"])?;
    thread::sleep(Duration::from_millis(10));
    if !rejected(&home, &expired) {
        return Ok(false);
    }
    let mut extended = decode(&expired)?;
    let later = Timestamp::now()
        .checked_add(SignedDuration::from_secs(60))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    extended["expires_at"] = json!(later.to_string());
    Ok(rejected(&home, &encode(&extended)?))
}

fn identity(index: usize) -> Result<String> {
    let endpoint = replicas::endpoints()
        .get(index)
        .cloned()
        .ok_or_else(|| Error::Invalid("replica index".into()))?;
    let document: Value = serde_json::from_slice(&fs::read(endpoint.home.join("credentials.json"))?)?;
    document["enrollment"]["replica_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("replica identity".into()))
}

fn cursor_of(index: usize) -> Option<(String, u64)> {
    let endpoint = replicas::endpoints().get(index).cloned()?;
    Client::cursor(&endpoint.client())
        .ok()
        .map(|c| (c.epoch, c.sequence))
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let gateway = gateway_client(world)?;
    let observed = Observed::new(gateway, GATEWAY_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Ok(());
    }
    if !automatic_healthy(world, &observed, evidence) {
        evidence.check("automatic_sync_guarantees", false, "healthy probe failed");
        return Ok(());
    }
    if !pairing_checks(world).unwrap_or(false) {
        evidence.check(
            "wrong_expired_replayed_code_rejected",
            false,
            "a bad pairing code was accepted or a good one refused",
        );
        return Ok(());
    }
    evidence.check(
        "wrong_expired_replayed_code_rejected",
        true,
        "wrong, replayed, expired and extended codes are rejected",
    );
    let start = Instant::now();
    let revoked = identity(0)
        .and_then(|id| world.gateway_cli(&["revoke", &id]))
        .is_ok();
    if !revoked {
        evidence.check("revoke_replica", false, "revoke failed");
        return Ok(());
    }
    let disconnected = await_until(1, || {
        replicas::endpoints().first().is_some_and(|endpoint| {
            replicas::status(endpoint).is_ok_and(|v| v["replica"]["state"] == "disconnected")
        })
    });
    if !disconnected {
        evidence.check(
            "live_replica_revoked_within_one_second",
            false,
            "replica stayed connected",
        );
        return Ok(());
    }
    evidence.check(
        "live_replica_revoked_within_one_second",
        start.elapsed() < Duration::from_secs(1),
        "replica disconnected within one second of revoke",
    );
    if world
        .control("/external-change", &json!({"key": "revoked-change-000000"}))
        .is_err()
    {
        evidence.check("post_revoke_change", false, "change rejected");
        return Ok(());
    }
    if world.control("/flush", &json!({})).is_err() {
        evidence.check("post_revoke_capture", false, "flush failed");
        return Ok(());
    }
    let before = cursor_of(0);
    thread::sleep(Duration::from_millis(1500));
    let after = cursor_of(0);
    evidence.check(
        "revoked_replica_no_further_data",
        before == after,
        "the revoked replica's cursor did not move",
    );
    Ok(())
}
