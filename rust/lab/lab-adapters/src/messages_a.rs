#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{
        Observed, await_until, converged, false_deletion_free, read_all, record_multiset,
    },
    history,
    http::Client,
    replicas,
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Command, time::Instant};

pub mod commands;
pub mod decode;

pub const READ_SCENARIOS: [&str; 4] = [
    "MSG-DB-REPLACED",
    "MSG-PERMISSION-LOSS",
    "MSG-INCOMING",
    "MSG-DECODE",
];
pub const SCENARIOS: [&str; 4] = READ_SCENARIOS;

fn action(world: &World, name: &str, key: &str) -> bool {
    world
        .control(
            "/messages",
            &json!({"action": name, "key": key, "count": 0}),
        )
        .is_ok()
}
fn generation(world: &World) -> String {
    world
        .control("/messages-status", &Value::Null)
        .ok()
        .and_then(|status| status["generation"].as_str().map(str::to_owned))
        .unwrap_or_default()
}
fn title(record: &Value) -> String {
    record["data"]["title"]
        .as_str()
        .or_else(|| record["id"].as_str())
        .unwrap_or_default()
        .to_owned()
}
fn message_ids(records: &[Value]) -> BTreeMap<String, String> {
    records
        .iter()
        .filter(|record| {
            record["collection"]
                .as_str()
                .is_some_and(|c| c.starts_with("messages."))
        })
        .map(|record| {
            (
                format!(
                    "{}/{}",
                    record["collection"].as_str().unwrap_or_default(),
                    title(record)
                ),
                record["id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}
fn message_records(records: &[Value]) -> BTreeMap<String, usize> {
    let kept: Vec<Value> = records
        .iter()
        .filter(|record| {
            record["collection"]
                .as_str()
                .is_some_and(|c| c.starts_with("messages."))
                && record["deleted"] != true
        })
        .cloned()
        .collect();
    record_multiset(&kept, false)
}
fn gateway_records(client: &Client) -> Vec<Value> {
    read_all(client, false, false)
        .map(|read| read.records)
        .unwrap_or_default()
}
fn seen(watcher: &Watcher, title: &str) -> Option<Instant> {
    watcher.first_seen_title("messages.message", title)
}
fn millis(later: Instant, earlier: Instant) -> f64 {
    later.saturating_duration_since(earlier).as_secs_f64() * 1000.0
}

pub fn start(world: &World, count: usize) -> Result<()> {
    replicas::start_with(world, count, history::SCOPE, 0)
}

pub fn read(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<bool> {
    let id = evidence.scenario.clone();
    let expected = id.trim_start_matches("MSG-").to_lowercase();
    if !READ_SCENARIOS.contains(&id.as_str()) || input["scenario"] != expected.as_str() {
        return Ok(false);
    }
    let seed = history::seed_count();
    let gateway = Client::new(world, history::SCOPE)?;
    let observed = Observed::new(gateway, history::SCOPE);
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    let settle = |world: &World| await_until(15, || converged(world, &observed));
    let seeded = world
        .control(
            "/messages",
            &json!({"action": "seed", "key": "", "count": seed}),
        )
        .is_ok();
    evidence.check("seed", seeded, "messages seeded in SimApple");
    let settled = await_until(15, || converged(world, &observed));
    evidence.check(
        "history_equals_truth",
        settled,
        "gateway and replica history equal SimApple truth",
    );
    if !settled {
        eprintln!("{}", crate::assertions::diagnose(world, &observed));
        return Ok(true);
    }
    match id.as_str() {
        "MSG-INCOMING" => {
            let mut gateway_ms = vec![];
            let mut replica_ms = vec![];
            let mut timeouts = 0;
            for index in 0..32 {
                let key = format!("incoming-{index:04}");
                if !action(world, "incoming", &key) {
                    timeouts += 1;
                    continue;
                }
                let at = Instant::now();
                let name = format!("[ab-test] {key}");
                let visible = await_until(1, || {
                    seen(&observers[0], &name).is_some() && seen(&observers[1], &name).is_some()
                });
                match (
                    visible,
                    seen(&observers[0], &name),
                    seen(&observers[1], &name),
                ) {
                    (true, Some(g), Some(r)) => {
                        gateway_ms.push(millis(g, at));
                        replica_ms.push(millis(r, at));
                    }
                    _ => timeouts += 1,
                }
            }
            let worst = replica_ms.iter().copied().fold(0.0, f64::max);
            evidence.check(
                "all_messages_under_one_second",
                timeouts == 0 && replica_ms.len() == 32 && worst < 1000.0,
                &format!(
                    "{} samples, worst replica latency {worst:.0} ms",
                    replica_ms.len()
                ),
            );
        }
        "MSG-DECODE" => {
            let output = Command::new("bash")
                .arg("packaging/dev/check-decoders.sh")
                .output();
            let text = output
                .as_ref()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default();
            evidence.check(
                "decoder_fixture_checks",
                output.is_ok_and(|o| o.status.success())
                    && text == "all decoder checks passed\nall SourceSupport checks passed",
                "native decoder and SourceSupport checks",
            );
            let malformed = action(world, "malformed", "message-000001") && settle(world);
            evidence.check("malformed_observed", malformed, "malformed body observed");
            let records = gateway_records(&observed.replicas[0].client());
            let found = records.iter().any(|record| {
                record["collection"] == "messages.message"
                    && title(record) == "[ab-test] message-000001"
                    && record["data"]["decode_status"] == "undecoded"
                    && record["data"]["text"].is_null()
                    && record["data"]["raw_body_digest"]
                        .as_str()
                        .is_none_or(|digest| !digest.is_empty())
            });
            evidence.check(
                "undecoded_retains_digest_without_empty_text",
                found,
                "undecoded replica row keeps its digest and has no text",
            );
        }
        "MSG-DB-REPLACED" => {
            let before = gateway_records(&observed.gateway);
            let old = generation(world);
            let replaced = action(world, "db-replacement", "") && settle(world);
            evidence.check("replacement", replaced, "database replacement recaptured");
            let after = gateway_records(&observed.gateway);
            evidence.check(
                "generation_changed",
                !old.is_empty() && generation(world) != old,
                "Messages generation changed",
            );
            evidence.check(
                "stable_identifiers_no_false_deletion",
                message_records(&before) == message_records(&after)
                    && message_ids(&before) == message_ids(&after),
                "record identities and data survive the replacement",
            );
        }
        "MSG-PERMISSION-LOSS" => {
            let before = gateway_records(&observed.gateway);
            evidence.check(
                "permission_loss",
                action(world, "permission-loss", ""),
                "permission removed",
            );
            evidence.check(
                "native_absence_while_denied",
                action(world, "delete", "message-000001"),
                "native row removed while denied",
            );
            evidence.check(
                "native_insert_while_denied",
                action(world, "incoming", "during-denial"),
                "native row inserted while denied",
            );
            let missing = await_until(5, || {
                gateway_records(&observed.gateway).iter().any(|record| {
                    record["collection"] == "system.source"
                        && record["data"]["state"] == "permission_missing"
                })
            });
            evidence.check(
                "permission_state",
                missing,
                "source reports permission_missing",
            );
            let after = gateway_records(&observed.gateway);
            evidence.check(
                "no_false_absence",
                message_ids(&before) == message_ids(&after),
                "no record vanished while permission was missing",
            );
            let restored = action(world, "permission-restore", "") && settle(world);
            evidence.check(
                "permission_restore",
                restored,
                "permission restored and recaptured",
            );
        }
        _ => return Ok(false),
    }
    tail(world, &observed, &observers, evidence)?;
    Ok(true)
}

fn tail(
    world: &World,
    observed: &Observed,
    observers: &[Watcher],
    evidence: &mut Evidence,
) -> Result<()> {
    let ok = await_until(15, || converged(world, observed));
    evidence.check(
        "automatic_convergence_with_truth",
        ok,
        "gateway and replicas equal SimApple truth",
    );
    for (index, observer) in observers.iter().enumerate() {
        evidence.check(
            &format!("automatic_no_false_deletion_{index}"),
            false_deletion_free(world, observed, observer),
            "every observed delete is absent in SimApple truth",
        );
        evidence.check(
            &format!("automatic_transactional_feed_{index}"),
            observer.valid(),
            "every batch continued the previous watermark",
        );
    }
    for (index, client) in observed.clients().iter().enumerate() {
        let token = if index == 0 {
            world
                .gateway_cli(&["token", "create", "--scope", "system:read"])?
                .trim()
                .to_owned()
        } else {
            replicas::cli(
                &observed.replicas[index - 1].home,
                &["token", "create", "--scope", "system:read"],
            )?
        };
        let scoped = Client::at(client.address(), &token);
        let page = scoped.call(
            "records.query",
            &json!({"collection":"messages.message","limit":1000}),
            20,
        );
        evidence.check(
            &format!("automatic_no_scope_leak_{index}"),
            page.is_ok_and(|p| {
                p.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty))
            }),
            "system:read cannot read messages",
        );
    }
    Ok(())
}

pub fn messages_read(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if read(world, input, evidence)? {
        return Ok(());
    }
    history::messages_read(world, input, evidence)
}
