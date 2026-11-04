use crate::{
    assertions::{Observed, await_until, converged},
    capture_atomic::{SCOPE, start_world},
    frame_proxy::FrameProxy,
    http::Client,
    journey::records,
    labcore, replicas,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{fs, path::Path, time::Duration};

pub const SCENARIOS: [&str; 2] = ["CORE-REPLICA-CAPTURE-STAGING", "CORE-REPLICA-CAPTURE-LEGACY"];
const BACKLOG: usize = 80;
const CREDIT_WINDOW: u64 = 4 * 1024 * 1024;

pub(crate) fn sql(home: &Path, query: &str) -> i64 {
    Connection::open_with_flags(home.join("state.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|db| db.query_row(query, [], |row| row.get::<_, i64>(0)))
        .unwrap_or(-1)
}

fn sequence_of(client: &Client) -> (String, u64) {
    client
        .cursor()
        .map(|cursor| (cursor.epoch, cursor.sequence))
        .unwrap_or_default()
}

fn drop_legacy_journal(world: &World) -> bool {
    Connection::open(world.gateway_home.join("state.db"))
        .and_then(|db| db.execute_batch("DROP TABLE journal_commits"))
        .is_ok()
}

fn prune_history(world: &mut World, through: u64) -> bool {
    world.stop_gateway();
    let aged = Connection::open(world.gateway_home.join("state.db"))
        .and_then(|db| {
            db.execute(
                "UPDATE journal SET committed_at=unixepoch()-3600 WHERE seq<=?",
                [i64::try_from(through).unwrap_or(i64::MAX)],
            )
        })
        .is_ok();
    let args = ["--retention-rows", "1", "--retention-age", "1s"]
        .map(str::to_owned)
        .to_vec();
    let ok = aged
        && world.configure_gateway(vec![], args).is_ok()
        && world
            .control(
                "/capture",
                &json!({
                    "source": "reminders",
                    "generation": "retention-fixture",
                    "checkpoint": "1",
                    "observations": [{
                        "kind": "upsert",
                        "collection": "reminders.reminder",
                        "native_key": "retention-marker",
                        "observed_at": jiff::Timestamp::now().to_string(),
                        "data": {"title": "[ab-test capture] retention marker"}
                    }]
                }),
            )
            .is_ok()
        && await_until(15, || {
            sql(
                &world.gateway_home,
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor'",
            ) == i64::try_from(through).unwrap_or(-2)
        });
    world.stop_gateway();
    ok
}

struct Case<'a> {
    name: &'a str,
    evidence: &'a mut Evidence,
}
impl Case<'_> {
    fn add(&mut self, label: &str, ok: bool, detail: &str) -> bool {
        self.evidence
            .check(&format!("{}/{label}", self.name), ok, detail);
        ok
    }
}

#[allow(clippy::too_many_lines)]
fn run_case(world: &mut World, name: &str, evidence: &mut Evidence) -> Result<()> {
    let mut case = Case { name, evidence };
    let started = start_world(world, 1);
    if !case.add("world", started.is_ok(), "") {
        return Ok(());
    }
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let Some(endpoint) = replicas::endpoints().first().cloned() else {
        return Err(Error::Invalid("no replica".into()));
    };
    let replica = endpoint.client();
    if !await_until(15, || converged(world, &observed)) {
        case.add("initial_convergence", false, "");
        return Ok(());
    }
    if name == "legacy" {
        replicas::crash(0)?;
        let restarted = fs::write(endpoint.home.join("lab-capture-legacy"), b"")
            .map_err(Error::from)
            .and_then(|()| replicas::restart(0));
        if !case.add("old_protocol_replica_started", restarted.is_ok(), "") {
            return Ok(());
        }
    }
    let proxy = match world
        .peer_address()
        .and_then(|peer| FrameProxy::start(&world.gateway_home, &endpoint.home, &peer))
    {
        Ok(proxy) => proxy,
        Err(error) => {
            case.add("frame_probe", false, &error.to_string());
            return Ok(());
        }
    };
    proxy.capture_reset(name == "reset");
    world.retarget_proxy(Some(proxy.address()))?;
    world.partition()?;
    world.heal()?;
    if !await_until(15, || {
        proxy.connections() >= 1 && replicas::wait_connected(1).is_ok()
    }) {
        case.add("frame_probe", false, "no feed through the probe");
        proxy.close();
        return Ok(());
    }
    world.partition()?;
    let before = sequence_of(&replica);
    let before_records = records(&replica).map(|(_, all)| all.len());
    if !case.add("visible_baseline", before_records.is_ok(), "") {
        proxy.close();
        return Ok(());
    }
    let before_records = before_records.unwrap_or_default();
    for n in 0..BACKLOG {
        let data = json!({"title": format!("[ab-test capture] {n}"), "notes": "c".repeat(60_000)});
        if world
            .control(
                "/external-change",
                &json!({"key": format!("capture-{n}"), "data": data}),
            )
            .is_err()
        {
            case.add("backlog_created", false, "");
            proxy.close();
            return Ok(());
        }
        if world.control_for("/flush", &json!({}), 5).is_err() {
            case.add("backlog_flushed", false, "");
            proxy.close();
            return Ok(());
        }
    }
    let target = sequence_of(&gateway);
    world.stop_gateway();
    let upgraded = drop_legacy_journal(world) && world.start_gateway().is_ok();
    if !case.add("upgrade_coalesces_retained_legacy_history", upgraded, "") {
        proxy.close();
        return Ok(());
    }
    if name == "upgrade-crash" {
        let _ = fs::remove_file(endpoint.home.join("lab-capture-part-reached"));
        let armed = fs::write(endpoint.home.join("lab-capture-part-armed"), b"");
        if armed.is_err() {
            case.add("stage_boundary_armed", false, "");
            proxy.close();
            return Ok(());
        }
    }
    world.heal()?;
    if name == "upgrade-crash" {
        let reached = await_until(10, || {
            fs::read(endpoint.home.join("lab-capture-part-reached"))
                .is_ok_and(|raw| raw == b"partial_staged")
        });
        let staged = sql(&endpoint.home, "SELECT COUNT(*) FROM pending_changes");
        case.add(
            "partial_durable_on_disk",
            reached && staged > 0,
            &format!("staged rows at kill: {staged}"),
        );
        case.add(
            "partial_does_not_advance_visible_cursor",
            sequence_of(&replica) == before,
            "",
        );
        let visible = records(&replica).map(|(_, all)| all.len());
        case.add(
            "partial_staging_is_absent_from_ordinary_reads",
            visible.is_ok_and(|count| count == before_records),
            "",
        );
        if !reached {
            proxy.close();
            return Ok(());
        }
        replicas::crash(0)?;
        world.partition()?;
        let restarted = replicas::restart(0);
        if !case.add(
            "restart_while_ingress_partitioned",
            restarted.is_ok(),
            "",
        ) {
            proxy.close();
            return Ok(());
        }
        case.add(
            "staging_cleared_before_hello",
            sql(&endpoint.home, "SELECT COUNT(*) FROM pending_changes") == 0
                && sequence_of(&replica) == before,
            "",
        );
        world.heal()?;
    }
    let arrived = await_until(30, || {
        let now = sequence_of(&replica);
        now.0 == target.0 && now.1 >= target.1 && converged(world, &observed)
    });
    case.add(
        "backlog_converges",
        arrived && converged(world, &observed),
        "",
    );
    let seen = proxy.capture_probe();
    let status = replicas::status(&endpoint).unwrap_or_default();
    let detail = format!(
        "{name} partial_frames={} partial_bytes={}",
        seen.partial_frames, seen.partial_bytes
    );
    if name == "legacy" {
        case.add(
            "feature_not_accepted_for_old_replica",
            seen.legacy_offers > 0 && seen.atomic_welcomes == 0 && seen.legacy_welcomes > 0,
            &detail,
        );
        case.add(
            "old_protocol_receives_no_partial_frames",
            seen.partial_frames == 0 && seen.partial_fields == 0,
            &detail,
        );
    } else {
        case.add(
            "capture_atomic_negotiated",
            seen.atomic_offers > 0 && seen.atomic_welcomes > 0,
            &detail,
        );
        case.add(
            "staging_empty_after_install",
            sql(&endpoint.home, "SELECT COUNT(*) FROM pending_changes") == 0,
            "",
        );
        if name == "upgrade-crash" {
            case.add(
                "partial_credits_do_not_ack_uncommitted_cursor",
                seen.partial_ack_without_cursor > 0 && seen.partial_ack_with_cursor == 0,
                &detail,
            );
            case.add(
                "oversized_backlog_streams_beyond_credit_window",
                seen.partial_frames > 1 && seen.partial_bytes > CREDIT_WINDOW,
                &detail,
            );
            case.add(
                "resume_starts_at_capture_beginning",
                seen.hello_sequences.last() == Some(&before.1),
                &format!("hello sequences {:?} before {}", seen.hello_sequences, before.1),
            );
            let applied = sql(
                &endpoint.home,
                &format!(
                    "SELECT COUNT(*) FROM applied_batches WHERE after_seq={} AND through_seq>={}",
                    before.1, target.1
                ),
            );
            case.add(
                "legacy_backlog_applied_in_one_streamed_transaction",
                applied == 1,
                "",
            );
            let pruned = prune_history(world, target.1);
            let floor = sql(
                &world.gateway_home,
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor'",
            );
            case.add(
                "retention_advances_on_commit_boundary",
                pruned && floor == i64::try_from(target.1).unwrap_or(-2),
                &format!("retention target {} floor {floor}", target.1),
            );
            let remaining = sql(
                &world.gateway_home,
                "SELECT COUNT(*) FROM journal_commits WHERE through_seq<=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor')",
            );
            case.add("retention_prunes_commit_end_rows", pruned && remaining == 0, "");
        } else {
            let discarded = endpoint.home.join("lab-capture-discarded").exists();
            case.add(
                "reset_discards_pending_capture",
                seen.reset_frames == 1 && discarded,
                &detail,
            );
            let snapshots = status["replica"]["snapshots"].as_u64().unwrap_or(0);
            case.add(
                "resnapshot_never_interleaves_partial_apply",
                snapshots >= 2
                    && sql(&endpoint.home, "SELECT COUNT(*) FROM pending_changes") == 0,
                &format!("snapshots {snapshots}"),
            );
        }
    }
    proxy.close();
    Ok(())
}

pub fn capture_staging(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let cases: &[&str] = if evidence.scenario == "CORE-REPLICA-CAPTURE-LEGACY" {
        &["legacy"]
    } else {
        &["upgrade-crash", "reset"]
    };
    let mut resets = 0;
    for (index, name) in cases.iter().enumerate() {
        replicas::stop_all();
        let mut isolated = if index == 0 {
            None
        } else {
            Some(world.sibling(name)?)
        };
        let outcome = match isolated.as_mut() {
            Some(sibling) => run_case(sibling, name, evidence),
            None => run_case(world, name, evidence),
        };
        let current = isolated.as_ref().unwrap_or(world);
        resets += labcore::observer_resets(current);
        replicas::stop_all();
        if let Some(mut sibling) = isolated {
            sibling.close();
        }
        outcome?;
        std::thread::sleep(Duration::from_millis(50));
    }
    evidence.check(
        "no_oversized_capture_observer_resets",
        resets == 0,
        &format!("capture observer resets: {resets}"),
    );
    Ok(())
}
