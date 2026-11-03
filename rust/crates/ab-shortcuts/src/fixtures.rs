use crate::{
    Cli, Error, Input, MAX_BYTES, Output, Runner,
    clock::{self, Alarm, Pack as ClockPack, Readback, Request, Timer},
    focus::{self, SetArgs, Status},
    pack, parse_inventory,
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const FAKE_CLI: &str = r#"#!/bin/sh
if [ "$1" = list ]; then
 if [ -f "$(dirname "$0")/pack" ]; then
  echo 'AB Echo (00000000-0000-0000-0000-000000000001)'
  echo 'AB Clock (00000000-0000-0000-0000-000000000002)'
  echo 'AB Focus (00000000-0000-0000-0000-000000000003)'
  echo 'AB Pack v1 (00000000-0000-0000-0000-000000000004)'
  exit 0
 fi
 if [ -f "$(dirname "$0")/changed" ]; then
  echo 'Changed (00000000-0000-0000-0000-000000000002)'
 else
  echo 'Synthetic (00000000-0000-0000-0000-000000000001)'
 fi
 exit 0
fi
name="$2"
shift 2
input=""
output=""
while [ "$#" -gt 0 ]; do
 case "$1" in
 --input-path) input="$2"; shift 2;;
 --output-path) output="$2"; shift 2;;
 *) shift;;
 esac
done
case "$name" in
 fail) exit 7;;
 flood) yes synthetic > "$output";;
 stdout-flood) yes synthetic;;
 timeout) echo $$ > "$(dirname "$0")/pid"; sleep 30 & echo $! > "$(dirname "$0")/child"; wait;;
 slow) sleep 16; cat "$input" > "$output";;
 binary) printf '\000\001\377' > "$output";;
 *) cat "$input" > "$output";;
esac
"#;

#[derive(Debug, Default)]
pub struct Report {
    pub assertions: Vec<(String, bool)>,
    pub limited: Option<String>,
}

impl Report {
    pub fn add(&mut self, name: &str, passed: bool) {
        self.assertions.push((name.to_owned(), passed));
    }
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "assertions": self.assertions.iter().map(|(n, p)| serde_json::json!({"name": n, "passed": p})).collect::<Vec<_>>(),
            "limited": self.limited,
        })
    }
}

pub fn prepare(directory: &Path) -> Result<PathBuf, Error> {
    fs::create_dir_all(directory)?;
    let executable = directory.join("shortcuts");
    fs::write(&executable, FAKE_CLI)?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    Ok(executable)
}

fn input(shortcut: &str, data: Option<&[u8]>, timeout: Duration) -> Input {
    Input {
        shortcut: shortcut.into(),
        data: data.map(<[u8]>::to_vec),
        timeout,
        ..Input::default()
    }
}

fn process_gone(pid: i64) -> bool {
    pid > 0
        && std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| !o.status.success())
}

fn read_pid(path: &Path) -> i64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

pub fn echo_check(report: &mut Report, executable: &Path) {
    let cli = Cli {
        executable: Some(executable.to_path_buf()),
    };
    let data = b"[ab-test synthetic echo]";
    let result = cli.run(&input("AB Echo", Some(data), Duration::from_secs(20)));
    report.add(
        "bounded_runner_completed",
        result
            .as_ref()
            .is_ok_and(|o| o.exit_status == 0 && !o.timed_out),
    );
    report.add("echo_output_matches", result.is_ok_and(|o| o.data == data));
}

pub fn capture_checks(report: &mut Report, directory: &Path, executable: &Path) {
    let cli = Cli {
        executable: Some(executable.to_path_buf()),
    };
    let first = cli.inventory(false, "");
    report.add(
        "initial_inventory",
        first.is_ok_and(|e| {
            e.len() == 1
                && e[0].name == "Synthetic"
                && e[0].identifier == "00000000-0000-0000-0000-000000000001"
        }),
    );
    report.add(
        "change_fixture",
        fs::write(directory.join("changed"), b"").is_ok(),
    );
    let second = cli.inventory(false, "");
    report.add(
        "changed_inventory",
        second.is_ok_and(|e| {
            e.len() == 1
                && e[0].name == "Changed"
                && e[0].identifier == "00000000-0000-0000-0000-000000000002"
        }),
    );
    report.add(
        "malformed_inventory_rejected",
        parse_inventory(b"ambiguous name\n").is_err(),
    );
    report.add(
        "duplicate_identifiers_rejected",
        parse_inventory(
            b"a (00000000-0000-0000-0000-000000000001)\nb (00000000-0000-0000-0000-000000000001)\n",
        )
        .is_err(),
    );
}

pub fn runner_checks(report: &mut Report, directory: &Path, executable: &Path) {
    let cli = Cli {
        executable: Some(executable.to_path_buf()),
    };
    let none = Duration::ZERO;
    for data in [
        &b"[ab-test synthetic]"[..],
        b"https://example.com/synthetic",
        &[0, 1, 255],
    ] {
        let result = cli.run(&input("echo", Some(data), none));
        report.add(
            "input_output_bytes",
            result.is_ok_and(|o| o.exit_status == 0 && o.data == data),
        );
    }
    let binary = cli.run(&input("binary", None, none));
    report.add("binary_output", binary.is_ok_and(|o| o.data == [0, 1, 255]));
    let failed = cli.run(&input("fail", None, none));
    report.add(
        "nonzero_exit_preserved",
        failed.is_ok_and(|o| o.exit_status == 7),
    );
    let started = Instant::now();
    let (result, error) = cli.run_detailed(&input("timeout", None, Duration::from_millis(100)));
    report.add(
        "timeout_bounded",
        matches!(error, Some(Error::DeadlineExceeded))
            && result.timed_out
            && started.elapsed() < Duration::from_secs(3),
    );
    let gone = process_gone(read_pid(&directory.join("pid")));
    report.add("timeout_process_reaped", result.exit_status == -1 && gone);
    std::thread::sleep(Duration::from_millis(100));
    report.add(
        "timeout_descendant_killed",
        process_gone(read_pid(&directory.join("child"))),
    );
    for name in ["flood", "stdout-flood"] {
        let started = Instant::now();
        let result = cli.run(&input(name, None, Duration::from_secs(10)));
        report.add(
            &format!("{name}_bounded_while_running"),
            result.is_err_and(|e| !matches!(e, Error::DeadlineExceeded))
                && started.elapsed() < Duration::from_secs(5),
        );
    }
    report.add(
        "option_injection_rejected",
        cli.run(&input("--help", None, none)).is_err(),
    );
    report.add(
        "oversized_input_rejected",
        cli.run(&input("echo", Some(&vec![0; MAX_BYTES + 1]), none))
            .is_err(),
    );
}

pub fn slow_run(executable: &Path) -> bool {
    let cli = Cli {
        executable: Some(executable.to_path_buf()),
    };
    cli.run(&input("slow", Some(b"x"), Duration::from_secs(20)))
        .is_ok_and(|o| o.data == b"x")
}

struct StubRunner {
    result: RefCell<Output>,
    error: RefCell<Option<String>>,
    seen: RefCell<Option<Input>>,
}

impl Runner for StubRunner {
    fn run(&self, input: &Input) -> Result<Output, Error> {
        *self.seen.borrow_mut() = Some(input.clone());
        if let Some(message) = self.error.borrow().as_ref() {
            return Err(Error::Message(message.clone()));
        }
        Ok(self.result.borrow().clone())
    }
}

fn tagged_alarm() -> Alarm {
    Alarm {
        id: "synthetic-alarm".into(),
        label: "[ab-test] clock".into(),
        time: "03:17".into(),
        repeat_days: Some(vec![]),
        enabled: true,
    }
}

fn clock_pack_request(operation: &str, alarm: &Alarm) -> (Request, Readback) {
    let mut request = Request {
        operation: operation.into(),
        ..Request::default()
    };
    let mut readback = Readback {
        version: 1,
        operation: operation.into(),
        alarms: Some(vec![alarm.clone()]),
        ..Readback::default()
    };
    match operation {
        "create_alarm" | "toggle_alarm" | "delete_alarm" => {
            let mut used = alarm.clone();
            if operation == "toggle_alarm" {
                used.enabled = false;
                readback.alarms = Some(vec![used.clone()]);
            }
            request.alarm = Some(used.clone());
            if operation == "create_alarm"
                && let Some(alarms) = readback.alarms.as_mut()
            {
                alarms[0].id = "clock-assigned-id".into();
            }
            if operation == "delete_alarm" {
                readback.alarms = Some(vec![]);
            }
        }
        "start_timer" => {
            request.duration = 90;
            request.label = "[ab-test] timer".into();
            readback.duration = 90;
            readback.label.clone_from(&request.label);
            readback.timer_running = true;
        }
        "world_time" => {
            request.city = "Chicago".into();
            readback.city = "Chicago".into();
            readback.local_time = "2026-10-28T03:17:00-05:00".into();
        }
        _ => {}
    }
    (request, readback)
}

fn bytes(readback: &Readback) -> Vec<u8> {
    serde_json::to_vec(readback).unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
pub fn clock_pack_checks(report: &mut Report, repo_root: &Path, scratch: &Path) {
    let stub = StubRunner {
        result: RefCell::new(Output {
            data: b"synthetic".to_vec(),
            exit_status: 0,
            timed_out: false,
        }),
        error: RefCell::new(None),
        seen: RefCell::new(None),
    };
    let pack = ClockPack::new(&stub);
    let raw = pack.inspect();
    report.add("fake_pack_runner", raw.is_ok_and(|r| r == b"synthetic"));
    let seen = stub.seen.borrow().clone();
    report.add(
        "bounded_named_pack",
        seen.as_ref().is_some_and(|i| {
            i.shortcut == "AB Clock" && i.timeout == Duration::from_secs(15) && i.data.is_none()
        }),
    );
    report.add(
        "completion_never_establishes_alarm_capture",
        matches!(pack.alarms(), Err(Error::ClockUnqualified)),
    );
    stub.result.borrow_mut().timed_out = true;
    report.add(
        "timeout_rejected",
        matches!(pack.inspect(), Err(Error::ClockUnqualified)),
    );
    {
        let mut result = stub.result.borrow_mut();
        result.timed_out = false;
        result.exit_status = 7;
    }
    report.add(
        "nonzero_exit_rejected",
        matches!(pack.inspect(), Err(Error::ClockUnqualified)),
    );
    {
        let mut result = stub.result.borrow_mut();
        result.exit_status = 0;
        result.data = vec![0; (32 << 10) + 1];
    }
    report.add(
        "oversize_output_rejected",
        matches!(pack.inspect(), Err(Error::ClockUnqualified)),
    );
    *stub.error.borrow_mut() = Some("context canceled".into());
    report.add(
        "runner_error_retained",
        matches!(pack.inspect(), Err(Error::Message(m)) if m == "context canceled"),
    );
    let alarm = tagged_alarm();
    for operation in [
        "alarms",
        "create_alarm",
        "toggle_alarm",
        "delete_alarm",
        "start_timer",
        "world_time",
    ] {
        let (request, mut readback) = clock_pack_request(operation, &alarm);
        *stub.error.borrow_mut() = None;
        stub.result.borrow_mut().data = bytes(&readback);
        report.add(
            &format!("structured_{operation}"),
            pack.exchange(&request).is_ok(),
        );
        let sent = stub
            .seen
            .borrow()
            .as_ref()
            .and_then(|i| i.data.as_ref())
            .and_then(|d| serde_json::from_slice::<Request>(d).ok());
        report.add(
            &format!("request_{operation}"),
            sent.is_some_and(|s| s.operation == operation),
        );
        let offline = ClockPack {
            runner: None,
            timeout: Duration::ZERO,
        };
        report.add(
            &format!("unqualified_live_{operation}"),
            matches!(offline.exchange(&request), Err(Error::ClockUnqualified)),
        );
        if operation == "start_timer" {
            readback.timer_running = false;
            stub.result.borrow_mut().data = bytes(&readback);
            report.add(
                "timer_requires_running_confirmation",
                matches!(pack.exchange(&request), Err(Error::ClockUnqualified)),
            );
        }
        if operation == "delete_alarm" {
            readback.alarms = Some(vec![alarm.clone()]);
            stub.result.borrow_mut().data = bytes(&readback);
            report.add(
                "delete_requires_absent_alarm",
                matches!(pack.exchange(&request), Err(Error::ClockUnqualified)),
            );
        }
        stub.result.borrow_mut().data = br#"{"version":1,"operation":"wrong"}"#.to_vec();
        report.add(
            &format!("mismatched_{operation}"),
            matches!(pack.exchange(&request), Err(Error::ClockUnqualified)),
        );
    }
    let personal = Alarm {
        label: "personal".into(),
        ..alarm
    };
    report.add(
        "untagged_alarm_rejected",
        pack.exchange(&Request {
            operation: "delete_alarm".into(),
            alarm: Some(personal),
            ..Request::default()
        })
        .is_err(),
    );
    let out = scratch.join("clock-build");
    report.add("temporary_build", fs::create_dir_all(&out).is_ok());
    let built = pack::build(&pack::BuildOptions {
        source: repo_root
            .join("packaging/shortcuts/ab-clock.json")
            .to_string_lossy()
            .into_owned(),
        out: out.clone(),
        unsigned: true,
        mode: "auto".into(),
    });
    report.add("repo_source_compiles", built.is_ok());
    report.add(
        "unsigned_artifact",
        out.join("ab-clock.unsigned.shortcut").is_file(),
    );
}

#[derive(Debug)]
pub struct ClockSimulation {
    alarms: RefCell<Vec<Alarm>>,
    timers: RefCell<Vec<Timer>>,
    recent: RefCell<Vec<Timer>>,
    capabilities: Vec<String>,
    pub operations: RefCell<Vec<String>>,
}

impl Default for ClockSimulation {
    fn default() -> Self {
        Self {
            alarms: RefCell::new(vec![Alarm {
                id: "existing-alarm".into(),
                label: "synthetic baseline".into(),
                time: "08:00".into(),
                repeat_days: Some(vec![]),
                enabled: false,
            }]),
            timers: RefCell::new(vec![Timer {
                id: "existing-timer".into(),
                label: "synthetic baseline".into(),
                duration: 600,
                running: false,
            }]),
            recent: RefCell::new(vec![Timer {
                id: "recent:1".into(),
                label: "synthetic recent".into(),
                duration: 300,
                running: false,
            }]),
            capabilities: vec!["timers".into(), "start_timer".into(), "delete_timer".into()],
            operations: RefCell::new(Vec::new()),
        }
    }
}

impl Runner for ClockSimulation {
    fn run(&self, input: &Input) -> Result<Output, Error> {
        let request: Request = input
            .data
            .as_deref()
            .and_then(|d| serde_json::from_slice(d).ok())
            .ok_or_else(|| Error::message("invalid synthetic request"))?;
        self.operations.borrow_mut().push(request.operation.clone());
        let previous = self.alarms.borrow().clone();
        let requested_alarm = request.alarm.clone().unwrap_or_default();
        let requested_timer = request.timer.clone().unwrap_or_default();
        match request.operation.as_str() {
            "create_alarm" => self.alarms.borrow_mut().push(Alarm {
                id: "created-alarm".into(),
                ..requested_alarm
            }),
            "toggle_alarm" => {
                for alarm in self.alarms.borrow_mut().iter_mut() {
                    if alarm.id == requested_alarm.id {
                        *alarm = requested_alarm.clone();
                    }
                }
            }
            "delete_alarm" => self
                .alarms
                .borrow_mut()
                .retain(|a| a.id != requested_alarm.id),
            "start_timer" => {
                self.timers.borrow_mut().push(Timer {
                    id: "created-timer".into(),
                    label: request.label.clone(),
                    duration: request.duration,
                    running: true,
                });
                self.recent.borrow_mut().push(Timer {
                    id: "recent:2".into(),
                    label: request.label.clone(),
                    duration: request.duration,
                    running: false,
                });
            }
            "delete_timer" => self
                .timers
                .borrow_mut()
                .retain(|t| t.id != requested_timer.id),
            "delete_recent_timer" => self
                .recent
                .borrow_mut()
                .retain(|t| t.id != requested_timer.id),
            "alarms" | "timers" | "recent_timers" | "capabilities" => {}
            _ => return Err(Error::message("unsupported synthetic operation")),
        }
        let recent_view = matches!(
            request.operation.as_str(),
            "recent_timers" | "delete_recent_timer"
        );
        let response = Readback {
            version: 1,
            operation: request.operation.clone(),
            alarms: Some(
                if matches!(request.operation.as_str(), "toggle_alarm" | "delete_alarm") {
                    previous
                } else {
                    self.alarms.borrow().clone()
                },
            ),
            timers: Some(if recent_view {
                self.recent.borrow().clone()
            } else {
                self.timers.borrow().clone()
            }),
            capabilities: Some(self.capabilities.clone()),
            timer_running: request.operation == "start_timer",
            duration: request.duration,
            label: request.label.clone(),
            ..Readback::default()
        };
        Ok(Output {
            data: serde_json::to_vec(&response)?,
            exit_status: 0,
            timed_out: false,
        })
    }
}

fn run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("run_{nanos:x}")
}

fn alarm_present(alarms: &[Alarm], id: &str) -> bool {
    alarms.iter().any(|a| a.id == id)
}

fn alarm_matches(alarms: &[Alarm], expected: &Alarm) -> bool {
    alarms.iter().find(|a| a.id == expected.id) == Some(expected)
}

fn same_alarms(actual: &[Alarm], expected: &[Alarm]) -> bool {
    actual.len() == expected.len() && expected.iter().all(|a| alarm_matches(actual, a))
}

fn limited(report: &mut Report, reason: &str) {
    report.limited = Some(reason.to_owned());
}

fn op(name: &str) -> Request {
    Request {
        operation: name.into(),
        ..Request::default()
    }
}

#[allow(clippy::too_many_lines)]
pub fn clock_alarm_session(report: &mut Report, pack: &ClockPack) {
    let run = run_id();
    let (before, error) = pack.exchange_partial(&op("alarms"));
    report.add("alarm_baseline_readback", error.is_none());
    if error.is_some() {
        limited(report, "AB Clock did not return structured alarm fields.");
        return;
    }
    let before_alarms = before.alarms.unwrap_or_default();
    let twelve_hours = jiff::Zoned::now()
        .checked_add(jiff::SignedDuration::from_hours(12))
        .map_or_else(|_| "12:00".to_owned(), |z| z.strftime("%H:%M").to_string());
    let mut alarm = Alarm {
        id: run.clone(),
        label: format!("[ab-test] {run}"),
        time: twelve_hours,
        repeat_days: Some(vec![1, 3]),
        enabled: true,
    };
    if before_alarms.iter().any(|a| a.label == alarm.label) {
        limited(report, "A matching test alarm already exists.");
        return;
    }
    let baseline: HashSet<String> = before_alarms.iter().map(|a| a.id.clone()).collect();
    let mut create = Request {
        operation: "create_alarm".into(),
        alarm: Some(alarm.clone()),
        ..Request::default()
    };
    let (created, error) = pack.exchange_partial(&create);
    let created_ids: Vec<String> = created
        .alarms
        .unwrap_or_default()
        .iter()
        .filter(|a| !baseline.contains(&a.id))
        .map(|a| a.id.clone())
        .collect();
    if created_ids.len() == 1 {
        alarm.id.clone_from(&created_ids[0]);
    }
    report.add("create_alarm_fields", error.is_none());
    if error.is_none() {
        let list = pack.exchange(&op("alarms"));
        report.add(
            "created_alarm_list_fields",
            list.is_ok_and(|r| alarm_matches(&r.alarms.unwrap_or_default(), &alarm)),
        );
        alarm.enabled = false;
        create.operation = "toggle_alarm".into();
        create.alarm = Some(alarm.clone());
        report.add("toggle_alarm_fields", pack.exchange(&create).is_ok());
        let list = pack.exchange(&op("alarms"));
        report.add(
            "toggled_alarm_list_fields",
            list.is_ok_and(|r| alarm_matches(&r.alarms.unwrap_or_default(), &alarm)),
        );
    }
    let observed = pack.exchange(&op("alarms"));
    let Ok(observed) = observed else {
        report.add("cleanup_inventory", false);
        return;
    };
    for candidate in observed.alarms.unwrap_or_default() {
        if !baseline.contains(&candidate.id)
            && (candidate.id == alarm.id || alarm.id == run && candidate.label == alarm.label)
        {
            alarm.id = candidate.id;
            break;
        }
    }
    create.operation = "delete_alarm".into();
    create.alarm = Some(alarm.clone());
    let deleted = pack.exchange(&create);
    let after = pack.exchange(&op("alarms"));
    let after_alarms = after.as_ref().ok().and_then(|r| r.alarms.clone());
    report.add(
        "cleanup_delete_readback",
        deleted.is_ok()
            && after_alarms
                .as_ref()
                .is_some_and(|a| !alarm_present(a, &alarm.id)),
    );
    if let Ok(after) = after {
        report.add(
            "cleanup_restores_baseline",
            same_alarms(&after.alarms.unwrap_or_default(), &before_alarms),
        );
    }
}

fn timer_map(timers: Option<Vec<Timer>>) -> HashMap<String, Timer> {
    timers
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect()
}

#[allow(clippy::too_many_lines)]
pub fn clock_timer_session(report: &mut Report, pack: &ClockPack) {
    let run = run_id();
    let capabilities = pack.exchange(&op("capabilities"));
    let qualified = capabilities.is_ok_and(|c| {
        let list = c.capabilities.unwrap_or_default();
        ["timers", "start_timer", "delete_timer"]
            .iter()
            .all(|n| list.iter().any(|c| c == n))
    });
    report.add("timer_readback_and_cleanup_qualified", qualified);
    if !qualified {
        limited(report, "Clock timer readback and cleanup are unavailable.");
        return;
    }
    let before = pack.exchange(&op("timers"));
    report.add("timer_baseline_readback", before.is_ok());
    let Ok(before) = before else { return };
    let recent = pack.exchange(&op("recent_timers"));
    report.add("recent_timer_baseline_readback", recent.is_ok());
    let Ok(recent) = recent else { return };
    let label = format!("[ab-test] {run}");
    let baseline_timers = before.timers.unwrap_or_default();
    let recent_timers = recent.timers.unwrap_or_default();
    if baseline_timers
        .iter()
        .chain(recent_timers.iter())
        .any(|t| t.label == label)
    {
        limited(report, "A matching test timer already exists.");
        return;
    }
    let baseline = timer_map(Some(baseline_timers));
    let recent_baseline = timer_map(Some(recent_timers));
    let started = pack.exchange(&Request {
        operation: "start_timer".into(),
        duration: 120,
        label: label.clone(),
        ..Request::default()
    });
    report.add("timer_running_duration_label_readback", started.is_ok());
    let observed = pack.exchange(&op("timers"));
    let matches = observed.as_ref().map_or(0, |r| {
        r.timers
            .iter()
            .flatten()
            .filter(|t| {
                !baseline.contains_key(&t.id) && t.label == label && t.duration == 120 && t.running
            })
            .count()
    });
    report.add(
        "created_timer_list_readback",
        observed.is_ok() && matches == 1,
    );
    cleanup_timers(report, pack, &label, &baseline, &recent_baseline);
}

fn cleanup_timers(
    report: &mut Report,
    pack: &ClockPack,
    label: &str,
    baseline: &HashMap<String, Timer>,
    recent_baseline: &HashMap<String, Timer>,
) {
    let observed = pack.exchange(&op("timers"));
    report.add("cleanup_timer_inventory", observed.is_ok());
    let Ok(observed) = observed else { return };
    for timer in observed.timers.unwrap_or_default() {
        if baseline.contains_key(&timer.id) || timer.label != label {
            continue;
        }
        let deleted = pack.exchange(&Request {
            operation: "delete_timer".into(),
            timer: Some(timer),
            ..Request::default()
        });
        report.add("cleanup_timer_deleted", deleted.is_ok());
    }
    let after = pack.exchange(&op("timers"));
    report.add(
        "cleanup_timer_baseline_restored",
        after.is_ok_and(|r| timer_map(r.timers) == *baseline),
    );
    let recents = pack.exchange(&op("recent_timers"));
    report.add("cleanup_recent_timer_inventory", recents.is_ok());
    let Ok(recents) = recents else { return };
    for timer in recents.timers.unwrap_or_default() {
        if recent_baseline.contains_key(&timer.id) || timer.label != label {
            continue;
        }
        let deleted = pack.exchange(&Request {
            operation: "delete_recent_timer".into(),
            timer: Some(timer),
            ..Request::default()
        });
        report.add("cleanup_recent_timer_deleted", deleted.is_ok());
    }
    let restored = pack.exchange(&op("recent_timers"));
    report.add(
        "cleanup_recent_timer_baseline_restored",
        restored.is_ok_and(|r| timer_map(r.timers) == *recent_baseline),
    );
}

struct FocusFixture {
    calls: Cell<u32>,
    input: RefCell<Option<Input>>,
    result: RefCell<Output>,
    focused: Cell<bool>,
}

impl Runner for FocusFixture {
    fn run(&self, input: &Input) -> Result<Output, Error> {
        self.calls.set(self.calls.get() + 1);
        *self.input.borrow_mut() = Some(input.clone());
        let args: SetArgs = serde_json::from_slice(input.data.as_deref().unwrap_or_default())?;
        let result = self.result.borrow().clone();
        if result.exit_status == 0 && !result.timed_out {
            self.focused.set(args.on);
        }
        Ok(result)
    }
}

pub fn focus_pack_checks(report: &mut Report) {
    let fixture = FocusFixture {
        calls: Cell::new(0),
        input: RefCell::new(None),
        result: RefCell::new(Output {
            data: Vec::new(),
            exit_status: 0,
            timed_out: false,
        }),
        focused: Cell::new(false),
    };
    let authorized = || -> Result<Status, Error> {
        Ok(Status {
            focused: Some(fixture.focused.get()),
            name: None,
            authorization: "authorized".into(),
        })
    };
    let pack = focus::Pack {
        runner: Some(&fixture),
        read_status: Some(&authorized),
    };
    let prior = fixture.focused.get();
    let until = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(60))
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default();
    let set = pack.set(&SetArgs {
        mode: "Do Not Disturb".into(),
        on: true,
        until,
    });
    report.add("set_readback", set.is_ok() && fixture.focused.get());
    report.add(
        "bounded_pack",
        fixture
            .input
            .borrow()
            .as_ref()
            .is_some_and(|i| i.shortcut == focus::NAME && i.timeout == Duration::from_secs(15)),
    );
    let restore = pack.set(&SetArgs {
        mode: "Do Not Disturb".into(),
        on: prior,
        until: String::new(),
    });
    report.add(
        "restore_readback",
        restore.is_ok() && fixture.focused.get() == prior,
    );
    fixture.result.borrow_mut().timed_out = true;
    report.add(
        "timeout_not_success",
        matches!(
            pack.set(&SetArgs {
                mode: "Do Not Disturb".into(),
                on: true,
                until: String::new()
            }),
            Err(Error::FocusUnqualified)
        ),
    );
    let unknown = || -> Result<Status, Error> {
        Ok(Status {
            focused: None,
            name: None,
            authorization: "not_determined".into(),
        })
    };
    let blocked = focus::Pack {
        runner: Some(&fixture),
        read_status: Some(&unknown),
    };
    let calls = fixture.calls.get();
    report.add(
        "unknown_status_never_dispatches",
        matches!(
            blocked.set(&SetArgs {
                mode: "Do Not Disturb".into(),
                on: true,
                until: String::new()
            }),
            Err(Error::FocusUnqualified)
        ) && fixture.calls.get() == calls,
    );
}

#[allow(clippy::too_many_lines)]
pub fn focus_compiled_checks(report: &mut Report, repo_root: &Path, scratch: &Path) {
    let out = scratch.join("focus-build");
    report.add("build_directory", fs::create_dir_all(&out).is_ok());
    let source = repo_root.join("packaging/shortcuts/ab-focus.json");
    let built = pack::build(&pack::BuildOptions {
        source: source.to_string_lossy().into_owned(),
        out,
        unsigned: true,
        mode: "auto".into(),
    });
    report.add("pack_compiles", built.is_ok());
    if built.is_err() {
        return;
    }
    let compiled = pack::compile_plist(&source);
    report.add("compiled_pack_read", compiled.is_ok());
    let Ok(compiled) = compiled else { return };
    let actions = compiled["WFWorkflowActions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let parameters = |action: &Value| action["WFWorkflowActionParameters"].clone();
    let until = actions
        .iter()
        .find(|a| {
            a["WFWorkflowActionIdentifier"] == "is.workflow.actions.getvalueforkey"
                && parameters(a)["WFDictionaryKey"] == "until"
        })
        .and_then(|a| parameters(a)["UUID"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let (mut bound, mut conditions) = (true, 0);
    for action in &actions {
        let p = parameters(action);
        if action["WFWorkflowActionIdentifier"] != "is.workflow.actions.conditional"
            || p["WFControlFlowMode"].as_i64().unwrap_or(0) != 0
        {
            continue;
        }
        conditions += 1;
        let input = &p["WFInput"];
        bound = bound
            && input["Type"] == "Variable"
            && input["Variable"]["WFSerializationType"] == "WFTextTokenAttachment"
            && input["Variable"]["Value"]["Type"] == "ActionOutput"
            && !until.is_empty()
            && input["Variable"]["Value"]["OutputUUID"] == until.as_str();
    }
    report.add("pack_conditional_input_bound", bound && conditions == 1);
    let (mut dates, mut date_bound) = (0, true);
    for action in &actions {
        let time = &parameters(action)["Time"];
        if time["WFSerializationType"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
        {
            continue;
        }
        dates += 1;
        let attachments = time["Value"]["attachmentsByRange"].as_object();
        let attachment = &time["Value"]["attachmentsByRange"]["{0, 1}"];
        date_bound = date_bound
            && time["WFSerializationType"] == "WFTextTokenString"
            && time["Value"]["string"] == "\u{fffc}"
            && attachments.is_some_and(|a| a.len() == 1)
            && attachment["Type"] == "ActionOutput"
            && attachment["OutputUUID"] == until.as_str();
    }
    report.add("pack_expiry_text_bound", date_bound && dates == 1);
}

pub fn clock_session_checks(report: &mut Report, timer: bool) {
    let simulation = ClockSimulation::default();
    let pack = ClockPack::new(&simulation);
    if timer {
        clock_timer_session(report, &pack);
    } else {
        clock_alarm_session(report, &pack);
    }
}

#[must_use]
pub fn valid_time(text: &str) -> bool {
    clock::valid_rfc3339(text)
}
