use crate::{
    assertions::{Observed, await_until, converged},
    journey::{GATEWAY_SCOPE, gateway_client, records},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};

pub const SCENARIOS: [&str; 1] = ["REM-DST"];

struct Case {
    name: &'static str,
    start: &'static str,
    all_day: bool,
}

const CASES: [Case; 10] = [
    Case {
        name: "spring_timed",
        start: "2026-03-08T01:30:00-05:00",
        all_day: false,
    },
    Case {
        name: "fall_timed",
        start: "2026-11-01T01:30:00-04:00",
        all_day: false,
    },
    Case {
        name: "spring_floating",
        start: "2026-03-08",
        all_day: true,
    },
    Case {
        name: "fall_floating",
        start: "2026-11-01",
        all_day: true,
    },
    Case {
        name: "spring_daily",
        start: "2026-03-07T09:00:00-05:00",
        all_day: false,
    },
    Case {
        name: "fall_daily",
        start: "2026-10-31T09:00:00-04:00",
        all_day: false,
    },
    Case {
        name: "spring_allday_daily",
        start: "2026-03-07",
        all_day: true,
    },
    Case {
        name: "fall_allday_daily",
        start: "2026-10-31",
        all_day: true,
    },
    Case {
        name: "utc_daily",
        start: "2026-03-07T14:00:00Z",
        all_day: false,
    },
    Case {
        name: "month_end",
        start: "2026-01-31",
        all_day: true,
    },
];

fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "gateway and SimApple peer ready");
    let list = world.control(
        "/external-change",
        &json!({
            "collection": "reminders.list",
            "key": "date-list",
            "data": {"title": "Bridge Test Dates", "writable": true}
        }),
    );
    evidence.check("reminder_list", list.is_ok(), "date list injected");
    let gateway = gateway_client(world)?;
    for case in &CASES {
        let alarm = if case.all_day {
            format!("{}T13:00:00Z", case.start)
        } else {
            case.start.to_owned()
        };
        let alarms = json!([
            {"absolute_date": alarm, "relative_offset": 0, "proximity": 0},
            {"absolute_date": null, "relative_offset": -1800, "proximity": 0}
        ]);
        let data = json!({
            "title": format!("Bridge Test {}", case.name),
            "due": case.start,
            "due_all_day": case.all_day,
            "completed": false,
            "list_id": reference("reminders.list", "date-list"),
            "alarms": alarms
        });
        let injected = world.control(
            "/external-change",
            &json!({"collection": "reminders.reminder", "key": case.name, "data": data}),
        );
        evidence.check(
            &format!("{}_fixture", case.name),
            injected.is_ok(),
            "fixture injected",
        );
        let title = format!("Bridge Test {}", case.name);
        let ok = await_until(5, || {
            let Ok((_, all)) = records(&gateway) else {
                return false;
            };
            let matched: Vec<&Value> = all
                .iter()
                .filter(|r| r["collection"] == "reminders.reminder" && r["data"]["title"] == title)
                .collect();
            matched.len() == 1
                && matched.iter().all(|r| {
                    r["data"]["due"] == case.start
                        && r["data"]["due_all_day"] == case.all_day
                        && r["data"]["alarms"] == alarms
                })
        });
        evidence.check(
            &format!("{}_readback", case.name),
            ok,
            "due, all-day flag and alarms round-trip unchanged",
        );
    }
    let observed = Observed::new(gateway, GATEWAY_SCOPE);
    evidence.check(
        "replica_converged",
        await_until(5, || converged(world, &observed)),
        "truth, gateway and replica agree",
    );
    Ok(())
}
