#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Observed, await_until, converged, false_deletion_free, read_all},
    http::Client,
    replicas,
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SCOPE: &str = "messages:read,contacts:read,notes:read,usage:read,system:read";
pub const SCENARIOS: [&str; 1] = ["MSG-HISTORY-IMPORT"];
const TARGET_TITLE: &str = "[ab-test] message-004095";
static SEED: AtomicU64 = AtomicU64::new(0);

pub fn messages(_: &mut World, input: &Value, _: &mut Evidence) -> Result<()> {
    SEED.store(input["count"].as_u64().unwrap_or(0), Ordering::SeqCst);
    Ok(())
}
pub fn seed_count() -> u64 {
    SEED.load(Ordering::SeqCst)
}
pub fn messages_truth(_: &mut World, _: &Value, _: &mut Evidence) -> Result<()> {
    Ok(())
}
fn action(world: &World, action: &str, key: &str, count: u64) -> bool {
    world
        .control(
            "/messages",
            &json!({"action": action, "key": key, "count": count}),
        )
        .is_ok()
}
fn blocked(world: &World) -> bool {
    world
        .control("/messages-status", &Value::Null)
        .is_ok_and(|status| status["bulk_blocked"] == true)
}
fn newer(data: &Value) -> bool {
    !data["read_at"].is_null() && !data["delivered_at"].is_null()
}
fn target_live(client: &Client) -> bool {
    read_all(client, false, false).is_ok_and(|read| {
        read.records.iter().any(|record| {
            record["collection"] == "messages.message"
                && record["data"]["title"] == TARGET_TITLE
                && newer(&record["data"])
        })
    })
}
pub fn messages_read(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if input["scenario"] != "history-import" || evidence.scenario != "MSG-HISTORY-IMPORT" {
        return crate::messages_b::read(world, input, evidence);
    }
    let seed = SEED.load(Ordering::SeqCst);
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway, SCOPE);
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    evidence.check(
        "seed",
        action(world, "seed", "", seed),
        "messages seeded in SimApple",
    );
    let settled = await_until(30, || converged(world, &observed));
    evidence.check(
        "history_equals_truth",
        settled,
        "gateway and replica history equal SimApple truth",
    );
    if !settled {
        eprintln!("{}", crate::assertions::diagnose(world, &observed));
        return Ok(());
    }
    evidence.check(
        "bulk_pause_installed",
        action(world, "bulk-pause", "message-004095", 0),
        "bulk capture held before the known target",
    );
    evidence.check(
        "bulk_started",
        action(world, "seed", "", 4096),
        "4096 synthetic rows queued",
    );
    evidence.check(
        "bulk_paused_before_known_target",
        await_until(30, || blocked(world)),
        "capture stalled at the chunk containing the target",
    );
    evidence.check(
        "live_mutation_during_import",
        action(world, "receipts", "message-004095", 0),
        "receipts applied while the bulk capture is paused",
    );
    evidence.check(
        "live_insert_during_import",
        action(world, "incoming", "during-import", 0),
        "incoming message inserted while the bulk capture is paused",
    );
    let clients = observed.clients();
    evidence.check(
        "live_visible_before_bulk_release",
        await_until(5, || clients.iter().all(target_live) && blocked(world)),
        "live receipts visible on every endpoint before the stale bulk is released",
    );
    evidence.check(
        "release_stale_bulk",
        action(world, "bulk-release", "", 0),
        "bulk capture released",
    );
    let feed_complete = await_until(60, || {
        observers
            .iter()
            .take(2)
            .all(|observer| observer.upserts_of("messages.message").0 >= 4098)
    });
    let reconverged = feed_complete && await_until(30, || converged(world, &observed));
    evidence.check(
        "backfill_revalidated_against_truth",
        reconverged,
        "gateway and replica equal SimApple truth after the stale bulk",
    );
    for (index, observer) in observers.iter().enumerate() {
        let mut live = false;
        let mut regressed = false;
        for data in observer.data_for_title("messages.message", TARGET_TITLE) {
            let current = newer(&data);
            regressed = regressed || live && !current;
            live = live || current;
        }
        evidence.check(
            &format!("older_backfill_never_regresses_live_value_{index}"),
            live && !regressed,
            "the live receipt value was never replaced by the older bulk value",
        );
    }
    tail(world, &observed, &observers, evidence)
}
pub fn tail(
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
