use ab_shortcuts::{
    Cli, Error, Input, MAX_BYTES, Runner,
    clock::{Alarm, Pack, Readback, Request},
    parse_inventory,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

const SCRIPT: &str = r#"#!/bin/sh
if [ "$1" = list ]; then
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
 binary) printf '\000\001\377' > "$output";;
 *) cat "$input" > "$output";;
esac
"#;

fn fixture(name: &str) -> (PathBuf, Cli) {
    let dir = std::env::temp_dir().join(format!("ab-sc-runner-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_default();
    let executable = dir.join("shortcuts");
    fs::write(&executable, SCRIPT).unwrap_or_default();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap_or_default();
    (
        dir,
        Cli {
            executable: Some(executable),
        },
    )
}

fn input(shortcut: &str, data: Option<&[u8]>) -> Input {
    Input {
        shortcut: shortcut.into(),
        data: data.map(<[u8]>::to_vec),
        ..Input::default()
    }
}

fn gone(pid: i32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| !o.status.success())
}

#[test]
fn inventory_parses_and_rejects() {
    assert!(parse_inventory(b"ambiguous name\n").is_err());
    assert!(
        parse_inventory(
            b"a (00000000-0000-0000-0000-000000000001)\nb (00000000-0000-0000-0000-000000000001)\n"
        )
        .is_err()
    );
    assert!(parse_inventory(b" (00000000-0000-0000-0000-000000000001)\n").is_err());
    let entries =
        parse_inventory(b"A (b) (0000000A-0000-0000-0000-00000000000b)\n\n").unwrap_or_default();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "A (b)");
    assert_eq!(parse_inventory(b"").unwrap_or_default().len(), 0);
}

#[test]
fn inventory_follows_the_cli() {
    let (dir, cli) = fixture("inventory");
    let first = cli.inventory(false, "").unwrap_or_default();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name, "Synthetic");
    fs::write(dir.join("changed"), b"").unwrap_or_default();
    let second = cli.inventory(false, "").unwrap_or_default();
    assert_eq!(second[0].name, "Changed");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn runner_preserves_bytes_exit_status_and_bounds() {
    let (dir, cli) = fixture("run");
    for data in [
        &b"[ab-test synthetic]"[..],
        b"https://example.com/synthetic",
        &[0, 1, 255],
    ] {
        let out = cli.run(&input("echo", Some(data)));
        assert!(out.is_ok_and(|o| o.exit_status == 0 && o.data == data));
    }
    assert_eq!(
        cli.run(&input("binary", None))
            .map(|o| o.data)
            .unwrap_or_default(),
        vec![0, 1, 255]
    );
    assert_eq!(
        cli.run(&input("fail", None)).map_or(-9, |o| o.exit_status),
        7
    );
    let started = Instant::now();
    let (result, error) = cli.run_detailed(&Input {
        timeout: Duration::from_millis(100),
        ..input("timeout", None)
    });
    assert!(
        matches!(error, Some(Error::DeadlineExceeded))
            && result.timed_out
            && result.exit_status == -1
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    let pid: i32 = fs::read_to_string(dir.join("pid"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(pid > 0 && gone(pid));
    std::thread::sleep(Duration::from_millis(100));
    let child: i32 = fs::read_to_string(dir.join("child"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(child > 0 && gone(child));
    for name in ["flood", "stdout-flood"] {
        let started = Instant::now();
        let error = cli.run(&Input {
            timeout: Duration::from_secs(10),
            ..input(name, None)
        });
        assert!(error.is_err_and(|e| !matches!(e, Error::DeadlineExceeded)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    assert!(cli.run(&input("--help", None)).is_err());
    assert!(
        cli.run(&input("echo", Some(&vec![0; MAX_BYTES + 1])))
            .is_err()
    );
    assert!(
        cli.run(&Input {
            timeout: Duration::from_secs(301),
            ..input("echo", None)
        })
        .is_err()
    );
    let _ = fs::remove_dir_all(dir);
}

struct Fixed(Vec<u8>);

impl Runner for Fixed {
    fn run(&self, _: &Input) -> Result<ab_shortcuts::Output, Error> {
        Ok(ab_shortcuts::Output {
            data: self.0.clone(),
            exit_status: 0,
            timed_out: false,
        })
    }
}

fn answer(readback: &Readback) -> Fixed {
    Fixed(serde_json::to_vec(readback).unwrap_or_default())
}

#[test]
#[allow(clippy::too_many_lines)]
fn clock_readback_rules_match_go() {
    let alarm = Alarm {
        id: "a1".into(),
        label: "[ab-test] clock".into(),
        time: "03:17".into(),
        repeat_days: Some(vec![]),
        enabled: true,
    };
    let request = Request {
        operation: "create_alarm".into(),
        alarm: Some(alarm.clone()),
        ..Request::default()
    };
    let mut readback = Readback {
        version: 1,
        operation: "create_alarm".into(),
        alarms: Some(vec![Alarm {
            id: "assigned".into(),
            time: "3:17".into(),
            ..alarm.clone()
        }]),
        ..Readback::default()
    };
    let runner = answer(&readback);
    let pack = Pack::new(&runner);
    assert!(request_ok(&pack, &request));
    readback.alarms = Some(vec![Alarm {
        enabled: false,
        ..alarm.clone()
    }]);
    assert!(!request_ok(&Pack::new(&answer(&readback)), &request));
    let untagged = Request {
        alarm: Some(Alarm {
            label: "personal".into(),
            ..alarm.clone()
        }),
        ..request.clone()
    };
    assert!(!request_ok(&pack, &untagged));
    let bad_time = Request {
        alarm: Some(Alarm {
            time: "25:00".into(),
            ..alarm.clone()
        }),
        ..request.clone()
    };
    assert!(!request_ok(&pack, &bad_time));
    let duplicate_days = Request {
        alarm: Some(Alarm {
            repeat_days: Some(vec![1, 1]),
            ..alarm
        }),
        ..request
    };
    assert!(!request_ok(&pack, &duplicate_days));
    let timer = Request {
        operation: "start_timer".into(),
        duration: 90,
        label: "[ab-test] timer".into(),
        ..Request::default()
    };
    let running = Readback {
        version: 1,
        operation: "start_timer".into(),
        duration: 90,
        label: timer.label.clone(),
        timer_running: true,
        ..Readback::default()
    };
    assert!(request_ok(&Pack::new(&answer(&running)), &timer));
    let stopped = Readback {
        timer_running: false,
        ..running
    };
    assert!(!request_ok(&Pack::new(&answer(&stopped)), &timer));
    let world = Request {
        operation: "world_time".into(),
        city: "Chicago".into(),
        ..Request::default()
    };
    let local = Readback {
        version: 1,
        operation: "world_time".into(),
        city: "Chicago".into(),
        local_time: "2026-10-28T03:17:00-05:00".into(),
        ..Readback::default()
    };
    assert!(request_ok(&Pack::new(&answer(&local)), &world));
    let skewed = Readback {
        local_time: "2026-10-28 03:17:00".into(),
        ..local
    };
    assert!(!request_ok(&Pack::new(&answer(&skewed)), &world));
    assert!(
        Pack {
            runner: None,
            timeout: Duration::ZERO
        }
        .exchange(&world)
        .is_err()
    );
    assert!(Pack::new(&Fixed(br#"{"version":1,"operation":"alarms","alarms":[{"id":"x","label":"l","time":"01:00","repeat_days":null,"enabled":true}]}"#.to_vec())).exchange(&Request { operation: "alarms".into(), ..Request::default() }).is_err());
}

fn request_ok(pack: &Pack, request: &Request) -> bool {
    pack.exchange(request).is_ok()
}
