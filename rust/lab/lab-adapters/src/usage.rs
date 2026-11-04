use crate::{
    assertions::{Observed, await_until, read_all},
    http::Client,
    imports::{usage, usage_tail, usage_totals},
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};

use crate::history::SCOPE;
pub const NATIVE_SCENARIOS: [&str; 2] = ["USE-SEGB-DECODE", "USE-ACTION-PROBE"];
pub const SCENARIOS: [&str; 3] = ["USE-LOCK-CLIP", "USE-READONLY", "USE-REMOTE-LAG"];
const APP: &str = "test.synthetic.alpha";
const BASE: f64 = 1_800_100_800.0;

fn event(offset: f64, starting: bool, bundle: &str) -> Value {
    json!({"at": BASE + offset, "starting": starting, "bundle": bundle})
}
fn send(world: &World, device: &str, platform: &str, events: &[Value]) -> bool {
    world
        .control(
            "/usage",
            &json!({"device": device, "platform": platform, "events": events}),
        )
        .is_ok()
}
pub(crate) fn session_restart(world: &mut World, evidence: &mut Evidence) {
    if std::env::var("AB_LAB_SESSION_RESTART").as_deref() != Ok("1")
        || !lab_world::session_sources()
            .iter()
            .any(|source| source == "usage")
    {
        return;
    }
    let before = world.session_checkpoint("usage");
    evidence.check("session_checkpoint_persisted", !before.is_empty(), "");
    evidence.check("session_restarted", world.restart_session().is_ok(), "");
    std::thread::sleep(std::time::Duration::from_secs(3));
    evidence.check(
        "session_resumed_without_rescan",
        !before.is_empty() && world.session_checkpoint("usage") == before,
        "",
    );
}
fn latest(client: &Client, device: &str) -> String {
    let Ok(read) = read_all(client, false, true) else {
        return String::new();
    };
    let id = read.aliases.iter().find_map(|alias| {
        (alias["collection"] == "usage.device" && alias["native_key"] == device)
            .then(|| alias["record_id"].as_str().map(str::to_owned))
            .flatten()
    });
    read.records
        .iter()
        .find(|record| {
            record["collection"] == "usage.device" && id.as_deref() == record["id"].as_str()
        })
        .and_then(|record| record["data"]["latest_event_at"].as_str())
        .unwrap_or_default()
        .to_owned()
}
fn totals(client: &Client, device: &str) -> (f64, f64) {
    usage_totals(client, device)
}
fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}
fn native_tool(name: &str) -> Option<PathBuf> {
    let directory = env::var("AB_LAB_NATIVE_OUTPUT").ok().or_else(|| {
        fs::read_to_string("native/.build/native-output-1")
            .ok()
            .map(|s| s.trim().to_owned())
    })?;
    let path = PathBuf::from(directory).join(name);
    path.is_file().then_some(path)
}
fn fixture_directory() -> PathBuf {
    PathBuf::from("lab/fixtures/usage")
}
pub fn decode(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let Some(binary) = native_tool("UsageChecks") else {
        evidence.check(
            "native_decoder_built",
            false,
            "UsageChecks binary unavailable on this host",
        );
        return Ok(());
    };
    let output = Command::new(binary)
        .arg(fixture_directory())
        .stdout(Stdio::null())
        .status();
    evidence.check(
        "scrubbed_segb_and_day_math",
        output.is_ok_and(|status| status.success()),
        "scrubbed SEGB decode and day math checks",
    );
    Ok(())
}
fn probe_truth(evidence: &mut Evidence) {
    let directory = env::temp_dir().join(format!("ab-usage-truth-checks-{}", std::process::id()));
    let passed = fs::create_dir_all(&directory).is_ok() && {
        let executable = directory.join("UsageTruthChecks");
        let built = Command::new("xcrun")
            .args([
                "swiftc",
                "-swift-version",
                "6",
                "-warnings-as-errors",
                "-parse-as-library",
                "lab/truth/native/Sources/AppleTruth/Usage.swift",
                "lab/fixtures/usage/probe-truth-checks.swift",
                "-o",
            ])
            .arg(&executable)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        built
            && Command::new(&executable)
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
    };
    let _ = fs::remove_dir_all(&directory);
    evidence.check(
        "independent_truth_handles_retired_peers",
        passed,
        "independent usage truth handles retired peers",
    );
}
pub fn cases(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "USE-IMPORT" => return usage(world, input, evidence),
        "USE-SEGB-DECODE" => return decode(world, input, evidence),
        "USE-ACTION-PROBE" => {
            if replicas::endpoints().is_empty() {
                replicas::start_with(world, 1, SCOPE, 0)?;
            }
        }
        _ if SCENARIOS.contains(&id.as_str()) => {}
        _ => {
            return Err(lab_world::Error::Invalid(format!(
                "usage not ported for {id}"
            )));
        }
    }
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let sent = send(
        world,
        "local",
        "mac",
        &[
            event(0.0, true, APP),
            event(600.0, true, "loginwindow"),
            event(3600.0, false, APP),
            event(0.0, true, APP),
            event(999.0, false, ""),
        ],
    );
    if !sent {
        evidence.check("usage_fixture", false, "usage fixture rejected");
        return Ok(());
    }
    let clipped = await_until(10, || {
        let (seconds, clipped) = totals(&gateway, "local");
        near(seconds, 600.0) && near(clipped, 3000.0)
    });
    evidence.check(
        "mac_lock_clipped",
        clipped,
        "lock time clipped from the focus session",
    );
    if !clipped {
        return Ok(());
    }
    session_restart(world, evidence);
    match id.as_str() {
        "USE-LOCK-CLIP" => lock_clip(world, &gateway, evidence),
        "USE-REMOTE-LAG" => {
            if !remote_lag(world, &gateway, evidence) {
                return Ok(());
            }
        }
        "USE-READONLY" => readonly(&gateway, evidence),
        _ => {}
    }
    usage_tail(&gateway, &observed, evidence)?;
    if id == "USE-ACTION-PROBE" {
        probe_truth(evidence);
    }
    Ok(())
}
fn lock_clip(world: &World, gateway: &Client, evidence: &mut Evidence) {
    evidence.check(
        "system_ui_batch",
        send(
            world,
            "remote-ui",
            "mac",
            &[
                event(0.0, true, APP),
                event(60.0, true, "com.apple.universalcontrol"),
                event(120.0, true, "test.synthetic.beta"),
                event(180.0, false, "test.synthetic.beta"),
            ],
        ),
        "system UI batch imported",
    );
    evidence.check(
        "system_ui_excluded_without_overlap",
        await_until(10, || {
            let (seconds, clipped) = totals(gateway, "remote-ui");
            near(seconds, 120.0) && near(clipped, 60.0)
        }),
        "system UI time is clipped without overlap",
    );
    evidence.check(
        "screensaver_batch",
        send(
            world,
            "local",
            "mac",
            &[
                event(4000.0, true, APP),
                event(4100.0, true, "com.apple.ScreenSaver.Engine"),
                event(4600.0, false, APP),
            ],
        ),
        "screensaver batch imported",
    );
    evidence.check(
        "screensaver_clipped",
        await_until(10, || {
            let (seconds, clipped) = totals(gateway, "local");
            near(seconds, 700.0) && near(clipped, 3500.0)
        }),
        "screensaver time is clipped",
    );
}
fn remote_lag(world: &World, gateway: &Client, evidence: &mut Evidence) -> bool {
    if !send(
        world,
        "remote",
        "iphone",
        &[event(-86_400.0, true, APP), event(-85_800.0, false, APP)],
    ) {
        evidence.check(
            "remote_initial_batch",
            false,
            "initial remote batch rejected",
        );
        return false;
    }
    let mut before = 0.0;
    let mut fresh = String::new();
    let settled = await_until(10, || {
        before = totals(gateway, "remote").0;
        fresh = latest(gateway, "remote");
        near(before, 600.0) && !fresh.is_empty()
    });
    evidence.check(
        "remote_older_freshness",
        settled && near(before, 600.0) && !fresh.is_empty(),
        "older remote batch carries freshness",
    );
    let started = Instant::now();
    if !send(
        world,
        "remote",
        "iphone",
        &[event(-85_000.0, true, APP), event(-84_700.0, false, APP)],
    ) {
        evidence.check(
            "remote_delayed_batch",
            false,
            "delayed remote batch rejected",
        );
        return false;
    }
    evidence.check(
        "past_day_revised",
        await_until(10, || {
            near(totals(gateway, "remote").0, 900.0) && latest(gateway, "remote") > fresh
        }),
        "delayed batch revises the past day",
    );
    evidence.check(
        "batch_live_under_one_second",
        started.elapsed().as_secs_f64() * 1000.0 < 1000.0,
        "delayed batch reaches the gateway within one second",
    );
    true
}
fn readonly(gateway: &Client, evidence: &mut Evidence) {
    for op in ["create", "update", "delete", "reset", "complete"] {
        for protocol in ["rest", "mcp"] {
            let operation = format!("usage.{op}");
            let raw = if protocol == "rest" {
                gateway.call_raw(&operation, &json!({}), 30)
            } else {
                gateway.call_mcp_raw(&operation, &json!({}), 30)
            };
            let unsupported = raw
                .ok()
                .flatten()
                .is_some_and(|(_, value)| value["error"]["code"] == "unsupported");
            evidence.check(
                &format!("usage_{op}_{protocol}_unsupported"),
                unsupported,
                "write operation is unsupported",
            );
        }
    }
}
