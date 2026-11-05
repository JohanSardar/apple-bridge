#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Observed, await_until, read_all, record_multiset, truth_matches},
    calendar::{SCOPE, STORE_SCOPE, text},
    http::{Client, random_id},
    replicas,
};
use jiff::Timestamp;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{env, fs, path::PathBuf, process::Command, time::Duration};

pub(crate) fn replicas_equal(observed: &Observed) -> bool {
    let Ok(gateway) = read_all(&observed.gateway, false, false) else {
        return false;
    };
    let expected = record_multiset(&gateway.records, true);
    observed.replicas.iter().all(|endpoint| {
        read_all(&endpoint.client(), false, false)
            .is_ok_and(|read| record_multiset(&read.records, true) == expected)
    })
}
pub(crate) fn convergence(world: &World, observed: &Observed) -> bool {
    truth_matches(world, observed) && replicas_equal(observed)
}
fn calendar_ref(key: &str) -> Value {
    json!({"$ref": {"collection": "calendar.calendar", "native_key": key}})
}
fn setup(world: &World, scope: &str) -> Result<Observed> {
    Ok(Observed::new(Client::new(world, scope)?, scope))
}
fn control(world: &World, route: &str, input: &Value) -> bool {
    world.control(route, input).is_ok()
}
fn calendar_control(world: &World, input: &Value) -> bool {
    control(world, "/calendar", input)
}

struct Case(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    bool,
    &'static [&'static str],
);

pub fn dst(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world started");
    let observed = setup(world, SCOPE)?;
    evidence.check(
        "calendar",
        calendar_control(
            world,
            &json!({"action": "calendar", "key": "test-calendar"}),
        ),
        "calendar created",
    );
    let cases = [
        Case(
            "spring_timed",
            "2026-03-08T01:30:00-05:00",
            "2026-03-08T03:30:00-04:00",
            "America/New_York",
            "",
            false,
            &["2026-03-08T01:30:00-05:00"],
        ),
        Case(
            "fall_timed",
            "2026-11-01T01:30:00-04:00",
            "2026-11-01T01:30:00-05:00",
            "America/New_York",
            "",
            false,
            &["2026-11-01T01:30:00-04:00"],
        ),
        Case(
            "spring_floating",
            "2026-03-08",
            "2026-03-09",
            "",
            "",
            true,
            &["2026-03-08"],
        ),
        Case(
            "fall_floating",
            "2026-11-01",
            "2026-11-02",
            "",
            "",
            true,
            &["2026-11-01"],
        ),
        Case(
            "spring_daily",
            "2026-03-07T09:00:00-05:00",
            "2026-03-07T10:00:00-05:00",
            "America/New_York",
            "daily",
            false,
            &[
                "2026-03-07T14:00:00Z",
                "2026-03-08T13:00:00Z",
                "2026-03-09T13:00:00Z",
            ],
        ),
        Case(
            "fall_daily",
            "2026-10-31T09:00:00-04:00",
            "2026-10-31T10:00:00-04:00",
            "America/New_York",
            "daily",
            false,
            &[
                "2026-10-31T13:00:00Z",
                "2026-11-01T14:00:00Z",
                "2026-11-02T14:00:00Z",
            ],
        ),
        Case(
            "spring_allday_daily",
            "2026-03-07",
            "2026-03-08",
            "America/New_York",
            "daily",
            true,
            &["2026-03-07", "2026-03-08", "2026-03-09"],
        ),
        Case(
            "fall_allday_daily",
            "2026-10-31",
            "2026-11-01",
            "America/New_York",
            "daily",
            true,
            &["2026-10-31", "2026-11-01", "2026-11-02"],
        ),
        Case(
            "utc_daily",
            "2026-03-07T14:00:00Z",
            "2026-03-07T15:00:00Z",
            "UTC",
            "daily",
            false,
            &[
                "2026-03-07T14:00:00Z",
                "2026-03-08T14:00:00Z",
                "2026-03-09T14:00:00Z",
            ],
        ),
        Case(
            "month_end",
            "2026-01-31",
            "2026-02-01",
            "",
            "monthly",
            true,
            &["2026-01-31", "2026-03-31", "2026-05-31"],
        ),
    ];
    for Case(name, start, end, zone, frequency, all_day, starts) in cases {
        let mut data = json!({"title": format!("Bridge Test {name}"), "start": start, "end": end,
            "all_day": all_day, "time_zone": zone, "calendar_id": calendar_ref("test-calendar")});
        if !frequency.is_empty() {
            data["recurrence"] = json!([{"frequency": frequency, "interval": 1, "count": 3}]);
        }
        let created = calendar_control(
            world,
            &json!({"action": "create", "key": name, "data": data}),
        );
        evidence.check(&format!("{name}_fixture"), created, "fixture created");
        let want = starts.len();
        let title = format!("Bridge Test {name}");
        let ok = await_until(5, || {
            let Ok(read) = read_all(&observed.gateway, false, false) else {
                return false;
            };
            let mut found = std::collections::BTreeSet::new();
            let mut matched = 0;
            for r in &read.records {
                if r["collection"] != "calendar.event" || text(r, "title") != title {
                    continue;
                }
                matched += 1;
                let got = &r["data"];
                if got["all_day"] != all_day || got["time_zone"] != zone {
                    return false;
                }
                let at = got["start"].as_str().unwrap_or_default().to_owned();
                if starts.contains(&at.as_str()) {
                    found.insert(at);
                }
                if frequency.is_empty() && got["end"] != end {
                    return false;
                }
            }
            found.len() == want && matched == want
        });
        evidence.check(&format!("{name}_readback"), ok, "events read back");
    }
    let ok = await_until(5, || convergence(world, &observed));
    evidence.check("replica_converged", ok, "truth, gateway and replica agree");
    Ok(())
}

fn participant(field: &str, value: Value) -> Value {
    let mut p = json!({"name": "Bridge Test", "url": "mailto:guest@example.com", "status": 1, "role": 1, "type": 1});
    p[field] = value;
    json!([p])
}
fn rule(field: &str, value: Value) -> Value {
    let mut r = json!({"frequency": "daily", "interval": 1, "count": 3, "end": null, "first_day_of_week": 1,
        "days_of_week": [], "days_of_month": [], "days_of_year": [], "weeks_of_year": [], "months": [], "positions": []});
    r[field] = value;
    json!([r])
}
fn at_offset(seconds: u64) -> Result<String> {
    let base: Timestamp = "2026-11-01T00:00:00Z"
        .parse()
        .map_err(|_| Error::Invalid("base".into()))?;
    base.checked_add(Duration::from_secs(seconds))
        .map(|t| t.to_string())
        .map_err(|e| Error::Invalid(e.to_string()))
}

pub fn stamp_fields(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world started");
    let observed = setup(world, SCOPE)?;
    let run = random_id("cal")?;
    let control = |action: &str, key: &str, data: Value| {
        calendar_control(
            world,
            &json!({"action": action, "key": key, "data": data, "force_stamp_match": action == "server_update"}),
        )
    };
    evidence.check(
        "calendar_created",
        control("calendar", "test-calendar", Value::Null),
        "calendar created",
    );
    evidence.check(
        "second_calendar_created",
        control(
            "calendar",
            "other-calendar",
            json!({"title": "Bridge Test second", "writable": true, "account": "test", "type": "local", "color": "#aabbcc"}),
        ),
        "second calendar created",
    );
    evidence.check(
        "verification_clock_started",
        calendar_control(
            world,
            &json!({"action": "clock", "now": "2026-11-01T00:00:00Z"}),
        ),
        "clock started",
    );
    let changes: Vec<(&str, &str, Value)> = vec![
        ("calendar_id", "calendar_id", calendar_ref("other-calendar")),
        ("title", "title", json!("[ab-test stamp] edited")),
        ("notes", "notes", json!("fixture notes")),
        ("location", "location", json!("Bridge Test room")),
        ("url", "url", json!("https://example.com/event")),
        ("start", "start", json!("2026-11-01T10:30:00Z")),
        ("end", "end", json!("2026-11-01T12:00:00Z")),
        ("all_day", "all_day", json!(true)),
        ("time_zone", "time_zone", json!("Europe/Paris")),
        ("is_exception", "is_exception", json!(true)),
        ("availability", "availability", json!(2)),
        ("status", "status", json!(2)),
        ("series_id", "series_id", json!("fixture-series")),
        (
            "occurrence_date",
            "occurrence_date",
            json!("2026-11-01T09:00:00Z"),
        ),
        (
            "attendee_name",
            "attendees",
            participant("name", json!("Bridge Test edited")),
        ),
        (
            "attendee_url",
            "attendees",
            participant("url", json!("mailto:other@example.com")),
        ),
        (
            "attendee_status",
            "attendees",
            participant("status", json!(2)),
        ),
        ("attendee_role", "attendees", participant("role", json!(2))),
        ("attendee_type", "attendees", participant("type", json!(2))),
        (
            "alarm_absolute",
            "alarms",
            json!([{"absolute_date": "2026-11-01T09:00:00Z", "relative_offset": 0}]),
        ),
        (
            "alarm_relative",
            "alarms",
            json!([{"absolute_date": null, "relative_offset": -600}]),
        ),
        (
            "recurrence_frequency",
            "recurrence",
            rule("frequency", json!("weekly")),
        ),
        (
            "recurrence_interval",
            "recurrence",
            rule("interval", json!(2)),
        ),
        ("recurrence_count", "recurrence", rule("count", json!(4))),
        (
            "recurrence_end",
            "recurrence",
            rule("end", json!("2026-12-01T10:00:00Z")),
        ),
        (
            "recurrence_first_day",
            "recurrence",
            rule("first_day_of_week", json!(2)),
        ),
        (
            "recurrence_weekday",
            "recurrence",
            rule("days_of_week", json!([{"day": 2, "week": 1}])),
        ),
        (
            "recurrence_monthday",
            "recurrence",
            rule("days_of_month", json!([2])),
        ),
        (
            "recurrence_yearday",
            "recurrence",
            rule("days_of_year", json!([3])),
        ),
        (
            "recurrence_yearweek",
            "recurrence",
            rule("weeks_of_year", json!([4])),
        ),
        ("recurrence_month", "recurrence", rule("months", json!([5]))),
        (
            "recurrence_position",
            "recurrence",
            rule("positions", json!([1])),
        ),
    ];
    let fast = ["location", "alarm_absolute", "alarm_relative"];
    let mut restart_key = String::new();
    for (index, (name, field, value)) in changes.iter().enumerate() {
        let key = random_id("stamp")?;
        restart_key.clone_from(&key);
        let title = format!("[ab-test {run}] {name}");
        let created = control(
            "create",
            &key,
            json!({"title": title, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}),
        );
        let baseline = control(
            "server_update",
            &key,
            json!({"attendees": participant("status", json!(1)),
                "alarms": [{"absolute_date": null, "relative_offset": -300}],
                "recurrence": rule("frequency", json!("daily")),
                "series_id": "baseline-series", "occurrence_date": "2026-11-01T10:00:00Z"}),
        );
        let offset = u64::try_from(index).map_err(|_| Error::Invalid("index".into()))? * 1800 + 900;
        let verification = calendar_control(
            world,
            &json!({"action": "clock", "now": at_offset(offset)?}),
        );
        let settled = await_until(3, || convergence(world, &observed));
        evidence.check(
            &format!("{name}_initial"),
            created && baseline && verification && settled,
            "initial capture matches truth",
        );
        let is_fast = fast.contains(name);
        let updated = calendar_control(
            world,
            &json!({"action": "server_update", "key": key, "data": {*field: value}, "force_stamp_match": !is_fast}),
        );
        let published = calendar_control(
            world,
            &json!({"action": "clock", "now": at_offset(offset + 1)?}),
        );
        if is_fast {
            let ok = await_until(1, || convergence(world, &observed));
            evidence.check(
                &format!("{name}_fast_path_without_modified_date"),
                updated && published && ok,
                "fast path captured the edit",
            );
        } else {
            evidence.check(
                &format!("{name}_retained_before_verification"),
                updated && published && !convergence(world, &observed),
                "edit retained until verification",
            );
        }
        let verified = calendar_control(
            world,
            &json!({"action": "clock", "now": at_offset(offset + 900)?}),
        );
        let ok = await_until(3, || convergence(world, &observed));
        evidence.check(
            &format!("{name}_periodic_verification_without_modified_date"),
            verified && ok,
            "periodic verification captured the edit",
        );
    }
    let edit = control(
        "server_update",
        &restart_key,
        json!({"notes": format!("[ab-test {run}] unpublished restart edit")}),
    );
    let count = u64::try_from(changes.len()).map_err(|_| Error::Invalid("count".into()))?;
    let pulse = calendar_control(
        world,
        &json!({"action": "clock", "now": at_offset(count * 1800 + 1)?}),
    );
    evidence.check(
        "metadata_unchanged_edit_unpublished_before_restart",
        edit && pulse && !convergence(world, &observed),
        "edit not published before restart",
    );
    evidence.check(
        "helper_restart_discards_in_memory_stamps",
        world.restart_peer().is_ok(),
        "helper restarted",
    );
    let ok = await_until(5, || convergence(world, &observed));
    evidence.check(
        "restart_full_capture_matches_truth",
        ok,
        "full capture after restart matches truth",
    );
    Ok(())
}

pub fn periodic_verification(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world started");
    let observed = setup(world, SCOPE)?;
    let run = random_id("cal")?;
    let ctl = |action: &str, mut values: Value| {
        values["action"] = json!(action);
        calendar_control(world, &values)
    };
    evidence.check(
        "calendar_created",
        ctl("calendar", json!({"key": "test-calendar"})),
        "calendar created",
    );
    evidence.check(
        "verification_clock_started",
        ctl("clock", json!({"now": "2026-11-01T00:00:00Z"})),
        "clock started",
    );
    let title = format!("[ab-test {run}] verification");
    evidence.check(
        "event_created",
        ctl(
            "create",
            json!({"key": "verification-event", "data": {"title": title, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}}),
        ),
        "event created",
    );
    let visible = |status: f64| {
        observed.clients().iter().all(|client| {
            let Ok(read) = read_all(client, false, false) else {
                return false;
            };
            let mut found = false;
            for record in &read.records {
                if record["data"]["title"] == title {
                    let attendees = record["data"]["attendees"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    found = status == 0.0 && attendees.is_empty();
                    if attendees.len() == 1 {
                        found = attendees[0]["status"].as_f64() == Some(status);
                    }
                }
            }
            found
        })
    };
    evidence.check(
        "initial_status_visible",
        await_until(5, || visible(0.0)),
        "initial status visible",
    );
    evidence.check(
        "server_status_changes_with_injected_stamp_match",
        ctl(
            "server_update",
            json!({"key": "verification-event", "force_stamp_match": true,
                "data": {"attendees": [{"name": "Bridge Test", "url": "mailto:guest@example.com", "status": 2, "role": 1, "type": 1}]}}),
        ),
        "server update applied",
    );
    let early = ctl("clock", json!({"now": "2026-11-01T00:14:59Z"})) && visible(0.0);
    evidence.check(
        "ordinary_scan_reuses_unchanged_stamp",
        early,
        "ordinary scan keeps the stale status",
    );
    evidence.check(
        "periodic_verification_triggered",
        ctl("clock", json!({"now": "2026-11-01T00:15:00Z"})),
        "verification clock reached",
    );
    evidence.check(
        "periodic_verification_rebuilds_unstamped_status",
        await_until(3, || visible(2.0)),
        "status rebuilt",
    );
    evidence.check(
        "periodic_verification_matches_independent_truth",
        await_until(3, || convergence(world, &observed)),
        "matches truth",
    );
    Ok(())
}

pub fn large_capture(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world started");
    let observed = setup(world, SCOPE)?;
    let run = random_id("cal")?;
    let tag = format!("[ab-test {run}]");
    evidence.check(
        "calendar_created",
        world
            .control_for(
                "/calendar",
                &json!({"action": "calendar", "key": "test-calendar"}),
                45,
            )
            .is_ok(),
        "calendar created",
    );
    let imported = world.control_for(
        "/calendar",
        &json!({"action": "create_many", "key": run, "count": 3000,
            "data": {"title": tag, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}}),
        45,
    );
    evidence.check(
        "single_change_imports_3000_events",
        imported.is_ok(),
        "3000 events imported in one change",
    );
    let mut target = Value::Null;
    let converged = await_until(45, || {
        for client in observed.clients() {
            let Ok(read) = read_all(&client, false, false) else {
                return false;
            };
            let mut count = 0;
            for record in &read.records {
                if record["collection"] == "calendar.event"
                    && text(record, "title").starts_with(&tag)
                {
                    count += 1;
                    target = record.clone();
                }
            }
            if count != 3000 {
                return false;
            }
        }
        true
    });
    evidence.check(
        "large_capture_converges",
        converged,
        "gateway and replica hold 3000 events",
    );
    if target["id"].as_str().is_none_or(str::is_empty) {
        return Ok(());
    }
    let command = json!({"command_id": random_id("cmd")?, "op": "calendar.update_event",
        "args": {"id": target["id"], "span": "future", "title": format!("{tag} continued")},
        "expected_version": target["version"], "wait_ms": 15000});
    let reply = observed
        .gateway
        .call_raw("calendar.update_event", &command, 40);
    let ok = matches!(&reply, Ok(Some((200, v))) if v["error"].is_null() && v["disposition"] == "locally_observed");
    evidence.check(
        "source_captures_after_large_change",
        ok,
        "update observed after a large capture",
    );
    let title = format!("{tag} continued");
    let reached = await_until(15, || {
        observed.clients().iter().all(|client| {
            read_all(client, false, false)
                .is_ok_and(|read| read.records.iter().any(|r| text(r, "title") == title))
        })
    });
    evidence.check(
        "following_change_reaches_replica",
        reached,
        "following change reached every endpoint",
    );
    Ok(())
}

fn external(world: &World, collection: &str, key: &str, data: &Value) -> bool {
    control(
        world,
        "/external-change",
        &json!({"collection": collection, "key": key, "data": data}),
    )
}
struct Page {
    events: Vec<Value>,
    calendars: Vec<Value>,
}
fn range(client: &Client, mcp: bool, input: &Value) -> Option<Page> {
    let reply = if mcp {
        client.call_mcp("calendar.range", input, 20)
    } else {
        client.call("calendar.range", input, 20)
    };
    let value = reply.ok().flatten()?;
    Some(Page {
        events: value["events"].as_array().cloned().unwrap_or_default(),
        calendars: value["calendars"].as_array().cloned().unwrap_or_default(),
    })
}
fn event_title(item: &Value) -> String {
    text(&item["event"], "title")
}

pub fn store_read(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world started");
    let observed = setup(world, STORE_SCOPE)?;
    for key in ["store-work", "store-home"] {
        let data = json!({"title": format!("Bridge Test {key}"), "color": "#007aff", "writable": true, "account": "demo-account", "type": "local"});
        if !external(world, "calendar.calendar", key, &data) {
            evidence.check("calendar_fixture", false, "calendar fixture failed");
            return Ok(());
        }
    }
    let entries = [
        (
            "store-a",
            "2026-03-08T06:30:00Z",
            "2026-03-08T08:30:00Z",
            "store-work",
            false,
        ),
        (
            "store-b",
            "2026-03-08T07:00:00Z",
            "2026-03-08T08:00:00Z",
            "store-work",
            false,
        ),
        (
            "store-c",
            "2026-03-08T08:30:00Z",
            "2026-03-08T09:00:00Z",
            "store-home",
            false,
        ),
        (
            "store-floating",
            "2026-03-08",
            "2026-03-10",
            "store-work",
            true,
        ),
        (
            "store-before",
            "2026-03-08T04:00:00Z",
            "2026-03-08T05:00:00Z",
            "store-work",
            false,
        ),
        (
            "store-after",
            "2026-03-09T04:00:00Z",
            "2026-03-09T05:00:00Z",
            "store-work",
            false,
        ),
    ];
    for (key, start, end, calendar, all_day) in entries {
        let data = json!({"title": key, "start": start, "end": end, "all_day": all_day, "time_zone": "America/New_York",
            "calendar_id": calendar_ref(calendar), "travel_time": 1200,
            "structured_location": {"title": "Bridge Test Station", "latitude": 40.7, "longitude": -74.0},
            "invitation_status": 2, "attachments": [], "conference_url": "https://example.com/meeting",
            "notes": if key == "store-a" { "train connection" } else { "" }});
        if !external(world, "calendar.event", key, &data) {
            evidence.check("event_fixture", false, "event fixture failed");
            return Ok(());
        }
    }
    let ok = await_until(5, || convergence(world, &observed));
    evidence.check("fixtures_converged", ok, "fixtures converged");
    let query = json!({"start": "2026-03-08T00:00:00-05:00", "end": "2026-03-09T00:00:00-04:00", "time_zone": "America/New_York"});
    for (endpoint_index, client) in observed.clients().iter().enumerate() {
        for mcp in [false, true] {
            let prefix = format!("{}_{endpoint_index}_", if mcp { "mcp" } else { "rest" });
            let check = |evidence: &mut Evidence, name: &str, ok: bool| {
                evidence.check(&format!("{prefix}{name}"), ok, name);
            };
            let Some(page) = range(client, mcp, &query) else {
                check(evidence, "range_available", false);
                continue;
            };
            check(evidence, "range_available", true);
            check(evidence, "half_open_dst_range", page.events.len() == 4);
            let position = |name: &str| page.events.iter().position(|e| event_title(e) == name);
            match (
                position("store-a"),
                position("store-b"),
                position("store-c"),
            ) {
                (Some(a), Some(b), Some(c)) => {
                    let (a, b, c) = (&page.events[a], &page.events[b], &page.events[c]);
                    check(
                        evidence,
                        "overlap_columns",
                        a["columns"] == 2
                            && b["columns"] == 2
                            && a["column"] != b["column"]
                            && a["overlap_group"] == b["overlap_group"],
                    );
                    check(
                        evidence,
                        "touching_intervals_separate",
                        c["columns"] == 1 && c["overlap_group"] != a["overlap_group"],
                    );
                }
                _ => check(evidence, "expected_occurrences", false),
            }
            if let Some(floating) = position("store-floating") {
                let data = &page.events[floating]["event"]["data"];
                check(
                    evidence,
                    "floating_civil_dates",
                    data["start"] == "2026-03-08" && data["end"] == "2026-03-10",
                );
            }
            let fields = position("store-a")
                .map_or(Value::Null, |a| page.events[a]["event"]["data"].clone());
            check(
                evidence,
                "store_fields_preserved",
                fields["travel_time"].as_f64() == Some(1200.0)
                    && fields["invitation_status"].as_f64() == Some(2.0)
                    && !fields["structured_location"].is_null(),
            );
            let calendar = page
                .calendars
                .iter()
                .find(|c| text(c, "title") == "Bridge Test store-work")
                .and_then(|c| c["id"].as_str())
                .unwrap_or_default()
                .to_owned();
            let mut filtered = query.clone();
            filtered["calendars"] = json!([calendar]);
            let selected = range(client, mcp, &filtered);
            check(
                evidence,
                "calendar_filter",
                !calendar.is_empty() && selected.is_some_and(|p| p.events.len() == 3),
            );
            filtered["text"] = json!("train");
            let selected = range(client, mcp, &filtered);
            check(
                evidence,
                "notes_search",
                selected.is_some_and(|p| p.events.len() == 1),
            );
            let invalid = json!({"start": query["end"], "end": query["start"]});
            check(
                evidence,
                "invalid_range_rejected",
                range(client, mcp, &invalid).is_none(),
            );
        }
    }
    let client = observed.gateway.clone();
    let updated = json!({"title": "store-b-updated", "start": "2026-03-08T10:00:00Z", "end": "2026-03-08T11:00:00Z",
        "all_day": false, "calendar_id": calendar_ref("store-work")});
    evidence.check(
        "cache_update_fixture",
        external(world, "calendar.event", "store-b", &updated),
        "update applied",
    );
    let ok = await_until(5, || {
        range(&client, false, &query).is_some_and(|page| {
            page.events
                .iter()
                .any(|e| event_title(e) == "store-b-updated")
                && page
                    .events
                    .iter()
                    .any(|e| event_title(e) == "store-a" && e["columns"] == 1)
        })
    });
    evidence.check(
        "cached_range_invalidates_after_update",
        ok,
        "cache invalidated",
    );
    evidence.check(
        "cache_delete_fixture",
        control(
            world,
            "/external-change",
            &json!({"collection": "calendar.event", "key": "store-b", "delete": true}),
        ),
        "delete applied",
    );
    let ok = await_until(5, || {
        range(&client, false, &query).is_some_and(|page| page.events.len() == 3)
    });
    evidence.check(
        "cached_range_invalidates_after_delete",
        ok,
        "cache invalidated",
    );
    let mut scoped = vec![(
        observed.gateway.clone(),
        world.gateway_cli(&["token", "create", "--scope", "system:read"]),
    )];
    for endpoint in &observed.replicas {
        scoped.push((
            endpoint.client(),
            replicas::cli(
                &endpoint.home,
                &["token", "create", "--scope", "system:read"],
            ),
        ));
    }
    for (endpoint, token) in scoped {
        let token = token.map(|t| t.trim().to_owned());
        for mcp in [false, true] {
            let page = token
                .as_ref()
                .ok()
                .and_then(|t| range(&Client::at(endpoint.address(), t), mcp, &query));
            evidence.check(
                &format!(
                    "{}_warm_cache_respects_scope",
                    if mcp { "mcp" } else { "rest" }
                ),
                page.is_some_and(|p| p.events.is_empty() && p.calendars.is_empty()),
                "system:read sees no calendar data",
            );
        }
    }
    if evidence.passed {
        range_perf(world, &observed, evidence)?;
        native_checks("CAL-STORE-NATIVE", evidence);
    }
    Ok(())
}

fn range_perf(world: &World, observed: &Observed, evidence: &mut Evidence) -> Result<()> {
    let mut records = vec![
        json!({"kind": "upsert", "collection": "calendar.calendar", "native_key": "store-perf",
        "data": {"title": "Bridge Test range perf", "writable": true, "color": "#007aff", "account": "demo"}}),
    ];
    let base: Timestamp = "2026-05-01T00:00:00Z"
        .parse()
        .map_err(|_| Error::Invalid("base".into()))?;
    for index in 0..1000u64 {
        let start = base
            .checked_add(Duration::from_secs(
                (index % 28) * 86400 + (8 + (index / 28) % 8) * 3600 + (index % 4) * 900,
            ))
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let end = start
            .checked_add(Duration::from_mins(30))
            .map_err(|e| Error::Invalid(e.to_string()))?;
        records.push(json!({"kind": "upsert", "collection": "calendar.event", "native_key": format!("store-perf-{index:04}"),
            "data": {"title": format!("Bridge Test {index:04}"), "start": start.to_string(), "end": end.to_string(),
                "all_day": false, "calendar_id": calendar_ref("store-perf")}}));
    }
    evidence.check(
        "month_1000_fixture",
        control(world, "/dataset", &json!({"records": records, "blobs": {}})),
        "1000 events loaded",
    );
    let ok = await_until(10, || convergence(world, observed));
    evidence.check("month_1000_converged", ok, "1000 events converged");
    let query = json!({"start": "2026-05-01T00:00:00Z", "end": "2026-06-01T00:00:00Z", "time_zone": "America/New_York"});
    for mcp in [false, true] {
        let name = if mcp { "mcp" } else { "rest" };
        let exactly = range(&observed.gateway, mcp, &query).is_some_and(|p| p.events.len() == 1000);
        evidence.check(
            &format!("{name}_month_exactly_1000"),
            exactly,
            "1000 events",
        );
        if !exactly {
            continue;
        }
        let mut values = vec![];
        let mut missing = 0;
        for _ in 0..100 {
            match observed
                .gateway
                .read_model_ms("calendar.range", &query, mcp)
            {
                Some(value) => values.push(value),
                None => missing += 1,
            }
        }
        values.sort_by(f64::total_cmp);
        let p95 = values.get(94).copied().unwrap_or(f64::MAX);
        evidence.check(
            &format!("{name}_month_handler_p95_under_10ms"),
            values.len() == 100 && missing == 0 && p95 <= 10.0,
            &format!("handler p95 {p95} ms"),
        );
    }
    Ok(())
}

pub fn native_checks(scenario: &str, evidence: &mut Evidence) {
    if scenario == "CAL-STORE-NATIVE" {
        crate::native_core::rust_tests(
            evidence,
            "store_reader_fields_and_blobs",
            "ab-native",
            &["--test", "calendar_store_oracle"],
        );
        return;
    }
    if scenario == "CAL-HINT-COVERAGE" {
        crate::native_core::rust_tests(
            evidence,
            "changed_push_survives_a_failed_pull",
            "ab-session",
            &["--test", "native_revalidation"],
        );
        return;
    }
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "CalendarChecks")
        .env("APPLE_BRIDGE_NATIVE_LAB", "1")
        .output();
    let built_ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_calendar_checks_build", built_ok, "native build");
    if !built_ok {
        return;
    }
    let directory = env::var("AB_LAB_NATIVE_OUTPUT")
        .ok()
        .or_else(|| fs::read_to_string("native/.build/native-output-1").ok())
        .map(|d| PathBuf::from(d.trim()));
    evidence.check(
        "native_checks_output_available",
        directory.is_some(),
        "native output directory",
    );
    let Some(directory) = directory else {
        return;
    };
    let flag = match scenario {
        "CAL-FUTURE-COUNT-MATH" => "--recurrence-bounds",
        _ => "--invalidated-event",
    };
    let output = Command::new(directory.join("CalendarChecks"))
        .arg(flag)
        .output();
    let success = output.as_ref().is_ok_and(|o| o.status.success());
    let name = if scenario == "CAL-FUTURE-COUNT-MATH" {
        "native_recurrence_bounds_and_postconditions"
    } else {
        "invalidated_calendar_object_rebuilds_without_trap"
    };
    evidence.check(name, success, "native check");
}
