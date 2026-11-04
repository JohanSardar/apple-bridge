use crate::{
    assertions::{Observed, await_until, converged},
    core_dials, core_frames,
    http::Client,
    journey::gateway_client,
    replicas::{self, Endpoint},
    robust::{change, replica_connected},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Instant};

pub const SCENARIOS: [&str; 5] = [
    "CORE-GAP-REJECT",
    "CORE-FEED-PIPELINED-ACKS",
    "CORE-CLOCK-DELAY-GATEWAY-DIALS",
    "CORE-BENCH",
    "CORE-DISK-FULL",
];
const BENCH_MESSAGES: u64 = 200_000;
const SIM_CHUNK: u64 = 64;
const BENCH_SECONDS: u64 = 300;
const CONTENDED_SECONDS: u64 = 900;
const BENCH_SCOPE: &str = "messages:read,reminders:read,system:read";

#[must_use]
pub fn handles(case: &str, scenario: &str) -> bool {
    matches!(
        (case, scenario),
        ("gap-reject", "CORE-GAP-REJECT")
            | ("pipelined-acks", "CORE-FEED-PIPELINED-ACKS")
            | (
                "clock-delay-gateway-dials",
                "CORE-CLOCK-DELAY-GATEWAY-DIALS"
            )
            | ("bench", "CORE-BENCH")
            | ("disk-full", "CORE-DISK-FULL")
    )
}
#[must_use]
pub fn network(scenario: &str) -> &'static str {
    if matches!(
        scenario,
        "CORE-GATEWAY-DIALS" | "CORE-CLOCK-DELAY-GATEWAY-DIALS"
    ) {
        "gateway_dials_loopback_fault_proxy"
    } else {
        "loopback"
    }
}
#[must_use]
pub fn observed(world: &World, case: &str, gateway: &Client) -> Observed {
    if case == "bench"
        && let Ok(admin) = bench_client(world)
    {
        return Observed::new(admin, "admin");
    }
    Observed::new(gateway.clone(), crate::journey::GATEWAY_SCOPE)
}
#[must_use]
pub fn tail_seconds(scenario: &str) -> u64 {
    if scenario == "CORE-BENCH" { 600 } else { 30 }
}
#[must_use]
pub fn skips_reset_check(scenario: &str) -> bool {
    matches!(
        scenario,
        "CORE-CLOCK-DELAY-GATEWAY-DIALS" | "CORE-DISK-FULL" | "CORE-BENCH"
    )
}

fn seed_default(world: &World) -> Result<()> {
    world.control("/seed", &json!({"count": 4}))?;
    let gateway = gateway_client(world)?;
    if await_until(10, || {
        crate::journey::records(&gateway).is_ok_and(|(_, all)| all.len() >= 6)
    }) {
        Ok(())
    } else {
        Err(Error::Invalid("default seed not captured".into()))
    }
}

pub fn start_world(world: &mut World, id: &str, count: usize) -> Result<()> {
    match id {
        "CORE-GAP-REJECT" | "CORE-FEED-PIPELINED-ACKS" => {
            if id == "CORE-FEED-PIPELINED-ACKS" {
                seed_default(world)?;
            }
            core_frames::install(world, &world.directory.join("replica0"))?;
            replicas::start(world, count)
        }
        "CORE-CLOCK-DELAY-GATEWAY-DIALS" => core_dials::start(world, count),
        "CORE-BENCH" => Ok(()),
        _ => replicas::start(world, count),
    }
}

pub fn run(case: &str, world: &mut World, observed: &Observed, evidence: &mut Evidence) -> bool {
    match case {
        "gap-reject" => gap_reject(world, observed, evidence),
        "pipelined-acks" => pipelined_acks(world, observed, evidence),
        "clock-delay-gateway-dials" => crate::core_robust::clock_delay_case(world, evidence),
        "bench" => bench(world, evidence),
        _ => disk_full(world, observed, evidence),
    }
}

fn fail(evidence: &mut Evidence, name: &str) -> bool {
    evidence.check(name, false, "step failed");
    false
}
fn cut(world: &World) -> Result<()> {
    let frame = core_frames::current()?;
    frame.arm();
    frame.reset_connections();
    world.partition()?;
    world.heal()
}

fn gap_reject(world: &World, observed: &Observed, evidence: &mut Evidence) -> bool {
    let Ok(frame) = core_frames::current() else {
        return fail(evidence, "frame_proxy_start");
    };
    if cut(world).is_err() {
        return fail(evidence, "frame_proxy_start");
    }
    frame.drop_next_batch();
    if !await_until(10, replica_connected) {
        return fail(evidence, "frame_proxy_connect");
    }
    if !change(world, "dropped-batch", 1) {
        return fail(evidence, "change_dropped_frame");
    }
    if !await_until(10, || frame.dropped() == 1) {
        return fail(evidence, "one_batch_dropped");
    }
    if !change(world, "after-gap", 10) {
        return fail(evidence, "change_after_gap");
    }
    if !await_until(15, || converged(world, observed)) {
        return fail(evidence, "gap_recovery");
    }
    evidence.check(
        "gap_rejected_connection_reopened",
        frame.connections() >= 2,
        &format!("feed connections after the gap: {}", frame.connections()),
    );
    true
}

fn cursor_sequence(client: &Client) -> Option<u64> {
    client.cursor().ok().map(|c| c.sequence)
}

fn pipelined_acks(world: &World, observed: &Observed, evidence: &mut Evidence) -> bool {
    let Ok(frame) = core_frames::current() else {
        return fail(evidence, "frame_proxy_connect");
    };
    frame.suppress_acks(false);
    let connected =
        cut(world).is_ok() && await_until(10, || frame.connections() >= 1 && replica_connected());
    if !connected {
        return fail(evidence, "frame_proxy_connect");
    }
    let initial = frame.connections();
    frame.suppress_acks(true);
    if !change(world, "unacknowledged-first", 1) {
        return fail(evidence, "first_unacknowledged_change");
    }
    if !await_until(5, || converged(world, observed)) {
        return fail(evidence, "first_batch_applied");
    }
    if !change(world, "unacknowledged-next", 20) {
        return fail(evidence, "following_changes");
    }
    if !await_until(5, || converged(world, observed)) {
        return fail(evidence, "later_batches_applied_before_any_ack");
    }
    evidence.check(
        "later_batches_applied_before_any_ack",
        true,
        "later batches applied while every ack was withheld",
    );
    let payload = "x".repeat(64 << 10);
    for index in 0..80 {
        let data = json!({"title": format!("[ab-test] credit-window-{index}"), "payload": payload});
        let key = format!("credit-window-{index}");
        if world
            .control("/external-change", &json!({"key": key, "data": data}))
            .is_err()
        {
            return fail(evidence, "fill_credit_window");
        }
    }
    if world.control_for("/flush", &json!({}), 10).is_err() {
        return fail(evidence, "credit_window_captures_committed");
    }
    let filled = await_until(10, || {
        frame.credit_high_water() > core_frames::BYTE_CREDIT - core_frames::MAX_FRAME
    });
    let high = frame.credit_high_water();
    evidence.check(
        "outstanding_metadata_within_four_mib",
        filled && high <= core_frames::BYTE_CREDIT,
        &format!("maximum outstanding metadata bytes: {high}"),
    );
    let replica = observed.replicas.first().map(Endpoint::client);
    let (gateway, replica) = (
        cursor_sequence(&observed.gateway),
        replica.as_ref().and_then(cursor_sequence),
    );
    evidence.check(
        "exhausted_credit_blocks_later_batches",
        matches!((gateway, replica), (Some(g), Some(r)) if g > r),
        &format!("gateway sequence {gateway:?}, replica sequence {replica:?}"),
    );
    evidence.check(
        "same_feed_connection",
        frame.connections() == initial,
        "the feed connection stayed open",
    );
    frame.suppress_acks(false);
    let _ = world.partition();
    let _ = world.heal();
    true
}

fn captures_committed(world: &World) -> u64 {
    world
        .control("/messages-status", &Value::Null)
        .ok()
        .and_then(|status| status["capture_count"].as_u64())
        .unwrap_or(0)
}
fn bench_client(world: &World) -> Result<Client> {
    let token = world
        .gateway_cli(&["token", "create", "--scope", "admin"])?
        .trim()
        .to_owned();
    Client::with_token(world, &token)
}
fn message_count(report: Option<Value>) -> Option<u64> {
    report?["records"]["messages.message"].as_u64()
}
fn doctor(home: Option<&Path>, world: &World) -> Option<Value> {
    let text = match home {
        Some(home) => replicas::cli(home, &["doctor"]).ok()?,
        None => world.gateway_cli(&["doctor"]).ok()?,
    };
    serde_json::from_str(&text).ok()
}
fn resident_bytes(pid: u32) -> Option<f64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .map(|kib| kib * 1024.0)
}
fn wal_bytes(home: &Path) -> Option<u64> {
    fs::metadata(home.join("state.db-wal"))
        .ok()
        .map(|m| m.len())
}
fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

fn bench_seconds() -> u64 {
    if std::env::var("APPLE_BRIDGE_LAB_CONTENDED_BENCH").is_ok_and(|v| v == "1") {
        CONTENDED_SECONDS
    } else {
        BENCH_SECONDS
    }
}

fn bench(world: &mut World, evidence: &mut Evidence) -> bool {
    let seconds = bench_seconds();
    let Ok(admin) = bench_client(world) else {
        return fail(evidence, "benchmark_token");
    };
    if replicas::start_with(world, 1, BENCH_SCOPE, 0).is_err() {
        return fail(evidence, "benchmark_replica");
    }
    let baseline = captures_committed(world);
    let start = Instant::now();
    if world
        .control_for(
            "/seed",
            &json!({"count": BENCH_MESSAGES, "collection": "messages.message"}),
            60,
        )
        .is_err()
    {
        return fail(evidence, "seed_two_hundred_thousand_messages");
    }
    let effect = Instant::now();
    let chunks = BENCH_MESSAGES.div_ceil(SIM_CHUNK);
    let flushed = await_until(seconds, || {
        captures_committed(world).saturating_sub(baseline) >= chunks
    }) && world.control_for("/flush", &json!({}), seconds).is_ok();
    eprintln!(
        "bench: committed_chunks={} of {chunks}",
        captures_committed(world).saturating_sub(baseline)
    );
    eprintln!(
        "bench: fixture_and_capture_ms={:.0} effect_done_to_gateway_capture_ms={:.0}",
        millis(start),
        millis(effect)
    );
    if !flushed {
        return fail(evidence, "capture_two_hundred_thousand_messages");
    }
    let Ok(head) = admin.cursor() else {
        return fail(evidence, "benchmark_gateway_cursor");
    };
    let replica = replicas::endpoints().into_iter().next();
    let replicated = await_until(seconds, || {
        replica.as_ref().is_some_and(|endpoint| {
            endpoint
                .client()
                .cursor()
                .is_ok_and(|c| c.epoch == head.epoch && c.sequence >= head.sequence)
        })
    });
    if !replicated {
        return fail(evidence, "replicate_two_hundred_thousand_messages");
    }
    let counts = [
        message_count(doctor(None, world)),
        replica
            .as_ref()
            .and_then(|e| message_count(doctor(Some(&e.home), world))),
    ];
    if counts.iter().any(|c| *c != Some(BENCH_MESSAGES)) {
        return fail(evidence, "benchmark_exact_message_counts");
    }
    let mut latencies = vec![];
    for _ in 0..100 {
        let began = Instant::now();
        let found = admin
            .call("search", &json!({"text": "needle", "limit": 1000}), 30)
            .ok()
            .flatten()
            .and_then(|r| r["records"].as_array().map(Vec::len));
        if found != Some(200) {
            return fail(evidence, "fts5_query_matches");
        }
        latencies.push(millis(began));
    }
    latencies.sort_by(f64::total_cmp);
    eprintln!(
        "bench: fts5_query_gateway p50_ms={:.2} p95_ms={:.2} max_ms={:.2}",
        latencies[49], latencies[94], latencies[99]
    );
    let gateway_rss = world.gateway_pid().and_then(resident_bytes);
    let replica_rss = replicas::pid(0).and_then(resident_bytes);
    let replica_home = replica.as_ref().map(|e| e.home.clone());
    eprintln!(
        "bench: gateway_rss_bytes={gateway_rss:?} replica_rss_bytes={replica_rss:?} gateway_wal_bytes={:?} replica_wal_bytes={:?}",
        wal_bytes(&world.gateway_home),
        replica_home.as_deref().and_then(wal_bytes)
    );
    if gateway_rss.is_none() || replica_rss.is_none() {
        return fail(evidence, "benchmark_resources");
    }
    evidence.check(
        "two_hundred_thousand_message_capture_replication_fts5",
        true,
        "200000 messages captured, replicated and searched",
    );
    true
}

fn run_tool(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}
fn cap_gateway_disk(world: &mut World) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(Error::Invalid("disk image needs macOS".into()));
    }
    world.stop_gateway();
    let image = world.directory.join("capacity.sparseimage");
    let mount = world.directory.join("disk");
    run_tool(
        "hdiutil",
        &[
            "create",
            "-size",
            "64m",
            "-fs",
            "HFS+",
            "-volname",
            "BridgeLab",
            "-type",
            "SPARSE",
            &image.to_string_lossy(),
        ],
    )?;
    fs::create_dir(&mount)?;
    run_tool(
        "hdiutil",
        &[
            "attach",
            &image.to_string_lossy(),
            "-mountpoint",
            &mount.to_string_lossy(),
            "-nobrowse",
        ],
    )?;
    let home = mount.join("gateway");
    run_tool(
        "cp",
        &[
            "-R",
            &world.gateway_home.to_string_lossy(),
            &home.to_string_lossy(),
        ],
    )?;
    world.gateway_home = home;
    world.restart_gateway()?;
    world.restart_peer()
}
fn fill_disk(mount: &Path) {
    let Ok(mut file) = fs::File::create(mount.join("filler")) else {
        return;
    };
    for size in [1 << 20, 1 << 16, 1 << 12, 1 << 9, 1] {
        let block = vec![0_u8; size];
        while std::io::Write::write_all(&mut file, &block).is_ok() {}
    }
    let _ = file.sync_all();
}
fn degraded_for_capacity(gateway: &Client) -> bool {
    gateway
        .call("sources.status", &json!({}), 10)
        .ok()
        .flatten()
        .is_some_and(|result| {
            result["records"].as_array().is_some_and(|records| {
                records
                    .iter()
                    .any(|r| r["data"]["state"] == "degraded" && r["data"]["reason"] == "capacity")
            })
        })
}

fn disk_full(world: &mut World, observed: &Observed, evidence: &mut Evidence) -> bool {
    let mount = world.directory.join("disk");
    if cap_gateway_disk(world).is_err() {
        return fail(evidence, "dedicated_capped_disk");
    }
    let Ok(gateway) = gateway_client(world) else {
        return fail(evidence, "capped_disk_ready");
    };
    let observed = Observed::new(gateway.clone(), &observed.gateway_scope);
    if !await_until(10, || converged(world, &observed)) {
        return fail(evidence, "capped_disk_ready");
    }
    fill_disk(&mount);
    if !change(world, "disk-full", 100) {
        return fail(evidence, "disk_full_external_changes");
    }
    if !await_until(15, || degraded_for_capacity(&gateway)) {
        return fail(evidence, "disk_full_degraded_capacity");
    }
    evidence.check(
        "real_volume_full_capture_degraded",
        true,
        "capture degraded with reason capacity on a full volume",
    );
    if fs::remove_file(mount.join("filler")).is_err() {
        return fail(evidence, "free_dedicated_disk");
    }
    true
}

pub fn finish_disk(world: &mut World, _: &Value, _: &mut Evidence) -> Result<()> {
    let mount = world.directory.join("disk");
    if mount.exists() {
        world.stop_gateway();
        let _ = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(&mount)
            .output();
    }
    Ok(())
}

pub fn native_stage(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if replicas::start(world, 1).is_err() {
        evidence.check("isolated_world_ready", false, "world did not start");
        return Ok(());
    }
    let gateway = Client::new(world, crate::blobs::SCOPE)?;
    crate::blobs::native_stage(world, evidence, &gateway, "CORE-BLOB-NATIVE-STAGE");
    Ok(())
}
pub fn native_stage_real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    native_stage(world, &Value::Null, evidence)
}
