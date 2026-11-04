use crate::{
    assertions::{Observed, await_until, converged},
    capture_staging::sql,
    http::{Client, random_id},
    labcore, replicas,
    watch::{Cursor, Watcher},
    web_agents::{create_agent, owner_session, post, text},
    webauthn::Owner,
};
use jiff::Timestamp;
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

const SCOPE: &str = "admin";
pub const SCENARIOS: [&str; 1] = ["CORE-REPLICA-CAPTURE-OBSERVERS"];
const COLLECTION: &str = "reminders.reminder";
const MARKER: &str = "[ab-test capture observer]";
const WIDTH: usize = 150;

fn observation(key: &str) -> (Value, Value) {
    let data = json!({"title": format!("{MARKER} {key}"), "text": "Bridge Test capture observer"});
    let item = json!({
        "kind": "upsert",
        "collection": COLLECTION,
        "native_key": key,
        "data": data,
        "observed_at": Timestamp::now().to_string()
    });
    (data, item)
}

fn oversized_capture(world: &World, gateway: &Client, prefix: &str) -> Option<Cursor> {
    let mut observations = vec![];
    for n in 0..WIDTH {
        let key = format!("{prefix}-{n}");
        let (data, item) = observation(&key);
        world
            .control(
                "/external-change",
                &json!({"key": key, "collection": COLLECTION, "data": data, "omit_capture": true}),
            )
            .ok()?;
        observations.push(item);
    }
    let before = gateway.cursor().ok()?;
    world
        .control(
            "/capture-atomic",
            &json!({"source": "reminders", "observations": observations}),
        )
        .ok()?;
    world.control_for("/flush", &json!({}), 5).ok()?;
    let through = gateway.cursor().ok()?;
    let width = sql(
        &world.gateway_home,
        &format!(
            "SELECT MAX(c.through_seq-COALESCE((SELECT MAX(p.through_seq) FROM journal_commits p WHERE p.epoch=c.epoch AND p.through_seq<c.through_seq),0)) FROM journal_commits c WHERE c.through_seq>{} AND c.through_seq<={}",
            before.sequence, through.sequence
        ),
    );
    let span = through.sequence.saturating_sub(before.sequence);
    (span >= WIDTH as u64 && width >= WIDTH as i64).then_some(through)
}

fn pending_state(replica: &Client, command: &str) -> (Option<bool>, String) {
    let page = replica
        .call(
            "records.query",
            &json!({"collection": "system.command", "limit": 1000}),
            15,
        )
        .ok()
        .flatten()
        .unwrap_or(Value::Null);
    let rows = page["records"].as_array().cloned().unwrap_or_default();
    let found = rows.iter().find(|row| row["id"] == command);
    (
        found.map(|row| row["data"]["awaiting_approval"] == true),
        format!("{} rows; {}", rows.len(), found.map_or_else(|| rows.first().map_or_else(String::new, ToString::to_string), ToString::to_string)),
    )
}

fn approvals_page(owner: &Owner) -> Option<String> {
    let reply = owner
        .request("GET", "/approvals", "application/json", b"")
        .ok()?;
    (reply.status == 200).then(|| String::from_utf8_lossy(&reply.body).into_owned())
}

fn effect(world: &World, id: &str) -> Option<u64> {
    Some(world.control("/effects", &Value::Null).ok()?[id].as_u64().unwrap_or(0))
}

fn approval_observer(world: &World, gateway: &Client, owner: &Owner, evidence: &mut Evidence) {
    let Some(replica) = replicas::endpoints().first().map(replicas::Endpoint::client) else {
        evidence.check("approval_authority", false, "no replica");
        return;
    };
    let observed = Observed::new(gateway.clone(), SCOPE);
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("approval_authority", false, "agent list");
        return;
    }
    let created = create_agent(
        world,
        "Bridge Test capture approval",
        "approve",
        "messages:write",
    );
    evidence.check("approval_agent_created", created.is_some(), "");
    let Some((_, token, _)) = created else {
        return;
    };
    let Ok(id) = random_id("cmd") else {
        return;
    };
    let command = json!({
        "command_id": id,
        "op": "messages.send",
        "args": {"handles": ["self@example.com"], "service": "iMessage", "text": format!("{MARKER} pending approval")},
        "wait_ms": 15000
    });
    let (code, pending) = post(world, &token, "/v1/op/messages.send", &command)
        .unwrap_or((0, Value::Null));
    let waiting = code == 200 && text(&pending, "state") == "awaiting_approval";
    evidence.check("approval_pending_before_observation", waiting, &pending.to_string());
    if !waiting {
        return;
    }
    let first = oversized_capture(world, gateway, "capture-approval");
    evidence.check("approval_oversized_capture_committed", first.is_some(), "");
    if first.is_none() {
        return;
    }
    let settled = await_until(10, || converged(world, &observed));
    evidence.check("approval_pending_replicated_after_capture", settled, "");
    let (state, detail) = pending_state(&replica, &id);
    evidence.check(
        "replica_pending_approval_survives_capture",
        state == Some(true),
        &detail,
    );
    let page = approvals_page(owner);
    evidence.check(
        "consent_page_rereads_pending_after_capture",
        page.as_ref().is_some_and(|page| page.contains(&id)),
        "",
    );
    evidence.check(
        "approval_capture_never_executes_pending_command",
        effect(world, &id) == Some(0),
        "",
    );
    let decided = owner.decide(&id, "approve");
    evidence.check(
        "approval_resolved_on_gateway",
        decided.is_ok_and(|(status, _)| status == 200),
        "",
    );
    let second = oversized_capture(world, gateway, "capture-resolved-approval");
    evidence.check(
        "resolved_approval_oversized_capture_committed",
        second.is_some(),
        "",
    );
    let settled = await_until(10, || converged(world, &observed));
    evidence.check("resolved_approval_capture_replicated", settled, "");
    let (state, detail) = pending_state(&replica, &id);
    evidence.check(
        "replica_resolved_approval_survives_capture",
        state == Some(false),
        &detail,
    );
    let page = approvals_page(owner);
    evidence.check(
        "resolved_approval_absent_from_consent_page",
        page.is_some_and(|page| !page.contains(&id)),
        "",
    );
}

struct Stream {
    reset: bool,
    batch: bool,
}

fn first_event(replica: &Client, token: &str, after: &Cursor) -> Result<Stream> {
    let mut child = Command::new("curl")
        .args(["--silent", "--no-buffer", "--max-time", "12"])
        .args(["-H", &format!("Authorization: Bearer {token}")])
        .arg(format!(
            "http://{}/v1/changes?after={}",
            replica.address(),
            after.event_id()
        ))
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| lab_world::Error::Invalid("stream".into()))?;
    let mut event = String::new();
    let mut seen = Stream {
        reset: false,
        batch: false,
    };
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if let Some(name) = line.strip_prefix("event: ") {
            name.clone_into(&mut event);
        } else if let Some(body) = line.strip_prefix("data: ") {
            if event == "reset" {
                let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                seen.reset = value["error"] == "reset";
            } else {
                seen.batch = event == "batch";
            }
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(seen)
}

fn title_of(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
fn observers(world: &World, gateway: &Client, evidence: &mut Evidence) {
    let observed = Observed::new(gateway.clone(), SCOPE);
    let Some(endpoint) = replicas::endpoints().first().cloned() else {
        return;
    };
    let replica = endpoint.client();
    let prefix = "capture-observer";
    let fixture = world
        .control(
            "/external-change",
            &json!({"key": format!("{prefix}-0"), "collection": COLLECTION, "data": {"title": format!("{MARKER} before")}}),
        )
        .is_ok()
        && world.control_for("/flush", &json!({}), 5).is_ok()
        && await_until(10, || converged(world, &observed));
    evidence.check("observer_fixture_converges", fixture, "");
    if !fixture {
        return;
    }
    let target = crate::journey::records(&replica)
        .map(|(_, all)| all)
        .unwrap_or_default()
        .into_iter()
        .find(|record| title_of(record) == format!("{MARKER} before"));
    evidence.check("observer_target", target.is_some(), "");
    let Some(target) = target else {
        return;
    };
    let watcher = Watcher::start(&replica, false);
    evidence.check("ordinary_sse_client_registered", watcher.is_ok(), "");
    let Ok(mut watcher) = watcher else {
        return;
    };
    let Ok(before) = replica.cursor() else {
        watcher.close();
        return;
    };
    let stream = std::thread::scope(|scope| {
        let reader = scope.spawn(|| first_event(&replica, &endpoint.token, &before));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let committed = oversized_capture(world, gateway, prefix);
        evidence.check("rest_observer_registered", true, "");
        evidence.check(
            "oversized_observer_capture_committed",
            committed.is_some(),
            "",
        );
        let applied = committed.as_ref().is_some_and(|through| {
            await_until(10, || {
                replica.cursor().is_ok_and(|local| {
                    local.epoch == through.epoch && local.sequence >= through.sequence
                })
            })
        });
        let result = reader.join().ok().and_then(Result::ok);
        (committed, applied, result)
    });
    let (committed, applied, result) = stream;
    let Some(through) = committed else {
        watcher.close();
        return;
    };
    evidence.check("oversized_observer_capture_applied", applied, "");
    if !applied {
        watcher.close();
        return;
    }
    let reached = await_until(5, || watcher.last().sequence >= through.sequence);
    let title = format!("{MARKER} {prefix}-0");
    let present = watcher.first_seen_title(COLLECTION, &title).is_some();
    let valid = watcher.valid() && watcher.resets() > 0;
    watcher.close();
    evidence.check(
        "ordinary_sse_client_resnapshots_without_partial_rows",
        reached && present && valid,
        &format!("reached={reached} present={present} valid={valid}"),
    );
    evidence.check(
        "rest_reset_before_any_capture_batch",
        result.is_some_and(|seen| seen.reset && !seen.batch),
        "",
    );
    let page = replica
        .call(
            "records.query",
            &json!({"collection": target["collection"], "limit": 1000}),
            20,
        )
        .ok()
        .flatten();
    let found = page.as_ref().is_some_and(|page| {
        page["records"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| row["id"] == target["id"] && title_of(row) == title)
        }) && page["cursor"]["sequence"].as_u64().unwrap_or(0) >= through.sequence
    });
    evidence.check("rest_resnapshot_has_committed_capture", found, "");
}

pub fn capture_observers(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    world.install_proxy()?;
    world.control("/seed", &json!({"count": 1}))?;
    replicas::start_with(world, 1, SCOPE, 0)?;
    let gateway = Client::new(world, SCOPE)?;
    let session = if world.gateway_cli(&["agent", "list"]).is_ok() {
        owner_session(world, None).ok()
    } else {
        evidence.check("authority", false, "agent list");
        return Ok(());
    };
    let Some(owner) = session else {
        evidence.check("owner_session", false, "owner enrollment");
        return Ok(());
    };
    approval_observer(world, &gateway, &owner, evidence);
    observers(world, &gateway, evidence);
    let resets = labcore::observer_resets(world);
    evidence.check(
        "capture_observer_resets_counted",
        resets > 0,
        &format!("capture observer resets: {resets}"),
    );
    Ok(())
}
