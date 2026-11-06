#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{
        Observed, await_until, false_deletion_free, gateway_aliases, read_all, record_integrity,
    },
    calendar_fixtures,
    http::{Client, random_id},
    replicas,
    watch::Watcher,
};
use jiff::{Span, Timestamp, tz::TimeZone};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const SCOPE: &str = "calendar:read,calendar:write,system:read";
pub const STORE_SCOPE: &str = "calendar:read,system:read";
pub const SCENARIOS: [&str; 25] = [
    "CAL-ALLDAY",
    "CAL-ALLDAY-FUTURE",
    "CAL-CREATE-EXTERNAL",
    "CAL-CRUD",
    "CAL-DST",
    "CAL-FUTURE-RULE-MISMATCH",
    "CAL-ID-CHURN",
    "CAL-LARGE-CAPTURE",
    "CAL-MOVED-OCCURRENCE",
    "CAL-MOVED-OUTSIDE-WINDOW",
    "CAL-PERIODIC-VERIFICATION",
    "CAL-PERMISSION-LOSS",
    "CAL-READONLY",
    "CAL-RECUR-ALL-REJECT",
    "CAL-RECUR-FUTURE",
    "CAL-RECUR-THIS",
    "CAL-SPLIT-ATOMIC-ALLDAY",
    "CAL-SPLIT-ATOMIC-TIMED",
    "CAL-SPLIT-FRAME-CAP",
    "CAL-STAMP-FIELDS",
    "CAL-WINDOW-SLIDE",
    "CAL-STORE-READ",
    "CAL-HINT-COVERAGE",
    "CAL-INVALIDATED-EVENT",
    "WEB-CAL-WRITE",
];
pub const NATIVE_ONLY: [&str; 3] = [
    "CAL-FUTURE-COUNT-MATH",
    "CAL-HINT-COVERAGE",
    "CAL-INVALIDATED-EVENT",
];
const START: &str = "2026-11-01T05:30:00Z";
const END: &str = "2026-11-01T07:30:00Z";

pub fn inject(scenario: &mut lab_runner::Scenario) {
    if scenario.id != "WEB-CAL-WRITE"
        || !scenario.given.is_empty()
        || !scenario.when.is_empty()
        || !scenario.then.is_empty()
    {
        return;
    }
    scenario.given.push(json!({"world": {"replicas": 1}}));
    scenario.when.push(json!({"calendar_journey": {}}));
    scenario.then.push(json!({"converge": {}}));
    scenario.cleanup.push(json!({"world": {}}));
}

pub fn calendar_journey(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "CAL-DST" => calendar_fixtures::dst(world, evidence),
        "CAL-STAMP-FIELDS" => calendar_fixtures::stamp_fields(world, evidence),
        "CAL-PERIODIC-VERIFICATION" => calendar_fixtures::periodic_verification(world, evidence),
        "CAL-LARGE-CAPTURE" => calendar_fixtures::large_capture(world, evidence),
        "CAL-STORE-READ" => calendar_fixtures::store_read(world, evidence),
        "CAL-FUTURE-COUNT-MATH" | "CAL-HINT-COVERAGE" | "CAL-INVALIDATED-EVENT" => {
            calendar_fixtures::native_checks(&id, evidence);
            Ok(())
        }
        _ if SCENARIOS.contains(&id.as_str()) => {
            let logic = match id.as_str() {
                "WEB-CAL-WRITE" => "CAL-CRUD",
                "CAL-SPLIT-ATOMIC-ALLDAY" => "CAL-ALLDAY-FUTURE",
                "CAL-SPLIT-ATOMIC-TIMED" => "CAL-RECUR-FUTURE",
                other => other,
            }
            .to_owned();
            let _ = protocol(world, evidence, &logic, false, "");
            if logic == "CAL-CRUD" && evidence.passed {
                let _ = protocol(world, evidence, &logic, true, "mcp_");
            }
            Ok(())
        }
        _ => Err(Error::Invalid(format!(
            "calendar_journey not ported for {id}"
        ))),
    }
}

pub(crate) fn text(value: &Value, key: &str) -> String {
    value["data"][key].as_str().unwrap_or_default().to_owned()
}
pub(crate) fn same_time(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    match (
        a.as_str().and_then(|s| s.parse::<Timestamp>().ok()),
        b.as_str().and_then(|s| s.parse::<Timestamp>().ok()),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}
fn version(record: &Value) -> u64 {
    record["version"].as_u64().unwrap_or(0)
}
fn id_of(record: &Value) -> String {
    record["id"].as_str().unwrap_or_default().to_owned()
}
fn start_of(record: &Value) -> String {
    record["data"]["start"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn civil_midnight(days: i64) -> Result<Timestamp> {
    let zone = TimeZone::system();
    let today = Timestamp::now().to_zoned(zone.clone()).date();
    let shifted = today
        .checked_add(Span::new().days(days))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    shifted
        .at(0, 0, 0, 0)
        .to_zoned(zone)
        .map(|z| z.timestamp())
        .map_err(|e| Error::Invalid(e.to_string()))
}
fn p95(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (sorted.len() * 95).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or(f64::MAX)
}

struct Split {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<String>>>,
    before: String,
    ready: bool,
    clients: Vec<Client>,
    tag: String,
}
fn snapshot(client: &Client, tag: &str) -> String {
    let Ok(read) = read_all(client, false, false) else {
        return "read_failed".into();
    };
    let mut identities: Vec<String> = read
        .records
        .iter()
        .filter(|r| r["collection"] == "calendar.event" && text(r, "title").starts_with(tag))
        .map(|r| {
            format!(
                "{}|{}|{}|{}",
                r["data"]["series_id"],
                r["data"]["occurrence_date"],
                r["data"]["start"],
                r["data"]["title"]
            )
        })
        .collect();
    identities.sort();
    identities.join("\n")
}
impl Split {
    fn start(clients: Vec<Client>, tag: &str) -> Self {
        let before = snapshot(&clients[0], tag);
        let ready = before != "read_failed"
            && await_until(5, || clients.iter().all(|c| snapshot(c, tag) == before));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = ready.then(|| {
            let (stop, clients, tag) = (stop.clone(), clients.clone(), tag.to_owned());
            thread::spawn(move || {
                let mut observed = vec![];
                loop {
                    for client in &clients {
                        observed.push(snapshot(client, &tag));
                    }
                    if stop.load(Ordering::SeqCst) {
                        return observed;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })
        });
        Self {
            stop,
            thread,
            before,
            ready,
            clients,
            tag: tag.to_owned(),
        }
    }
    fn finish(mut self) -> bool {
        if !self.ready {
            return false;
        }
        let after = snapshot(&self.clients[0], &self.tag);
        let converged = await_until(5, || {
            self.clients.iter().all(|c| snapshot(c, &self.tag) == after)
        });
        self.stop.store(true, Ordering::SeqCst);
        let observed = self
            .thread
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        converged
            && observed
                .iter()
                .all(|value| *value == self.before || *value == after)
    }
}
impl Drop for Split {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

struct Run<'a> {
    world: &'a mut World,
    evidence: &'a mut Evidence,
    prefix: &'a str,
    mcp: bool,
    tag: String,
    gateway: Client,
    observed: Observed,
    observers: Vec<Watcher>,
    last: Value,
    effects: BTreeMap<String, Instant>,
    latencies: BTreeMap<String, Vec<f64>>,
}
impl Run<'_> {
    fn add(&mut self, name: &str, ok: bool) {
        self.evidence
            .check(&format!("{}{name}", self.prefix), ok, name);
    }
    fn fail(&mut self, name: &str) -> Option<()> {
        self.add(name, false);
        None
    }
    fn endpoints(&self) -> Vec<Client> {
        self.observed.clients()
    }
    fn records_at(&self, client: &Client) -> Vec<Value> {
        let tag = &self.tag;
        let mut out: Vec<Value> = read_all(client, false, false)
            .map(|read| read.records)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r["collection"] == "calendar.event" && text(r, "title").contains(tag))
            .collect();
        out.sort_by_key(start_of);
        out
    }
    fn records(&self) -> Vec<Value> {
        self.records_at(&self.gateway)
    }
    fn await_count(&self, count: usize) -> bool {
        let clients = self.endpoints();
        await_until(5, || {
            clients.iter().all(|c| self.records_at(c).len() == count)
        })
    }
    fn calendar_id(&self) -> String {
        let Ok(aliases) = gateway_aliases(&self.gateway) else {
            return String::new();
        };
        let wanted = aliases
            .get("calendar/calendar.calendar/test-calendar")
            .cloned()
            .unwrap_or_default();
        read_all(&self.gateway, false, false)
            .map(|read| read.records)
            .unwrap_or_default()
            .iter()
            .find(|r| {
                r["collection"] == "calendar.calendar"
                    && text(r, "title") == "Bridge Test"
                    && id_of(r) == wanted
            })
            .map(id_of)
            .unwrap_or_default()
    }
    fn fixture(&mut self, action: &str, key: &str, data: &Value) -> Result<Value> {
        let started = Instant::now();
        let response = self.world.control(
            "/calendar",
            &json!({"action": action, "key": key, "data": data}),
        )?;
        if let Some(title) = data["title"].as_str() {
            let effect = response["effect_at"]
                .as_str()
                .and_then(|at| at.parse::<Timestamp>().ok())
                .and_then(|at| {
                    let age = Timestamp::now().duration_since(at);
                    Duration::try_from(age)
                        .ok()
                        .and_then(|age| Instant::now().checked_sub(age))
                })
                .unwrap_or(started);
            self.effects.insert(title.to_owned(), effect);
        }
        Ok(response)
    }
    fn latency(&mut self, title: &str) -> bool {
        if !await_until(3, || {
            self.observers
                .iter()
                .all(|o| o.first_seen_title("calendar.event", title).is_some())
        }) {
            return false;
        }
        let Some(effect) = self.effects.get(title).copied() else {
            return false;
        };
        for (index, observer) in self.observers.iter().enumerate() {
            let Some(seen) = observer.first_seen_title("calendar.event", title) else {
                return false;
            };
            if seen < effect {
                return false;
            }
            let key = if index == 0 {
                "effect_done→gateway_visible".to_owned()
            } else {
                format!("effect_done→replica_visible:replica{}", index - 1)
            };
            self.latencies
                .entry(key)
                .or_default()
                .push(seen.duration_since(effect).as_secs_f64() * 1000.0);
        }
        true
    }
    fn status(&self, id: &str) -> Option<Value> {
        let reply = if self.mcp {
            self.gateway
                .call_mcp_raw("commands.status", &json!({"command_id": id}), 20)
        } else {
            self.gateway
                .call_raw("commands.status", &json!({"command_id": id}), 20)
        };
        reply
            .ok()
            .flatten()
            .map(|(_, value)| value)
            .filter(|value| value["command"].is_object())
    }
    fn call(&self, op: &str, body: &Value) -> Result<Option<(u16, Value)>> {
        if self.mcp {
            self.gateway.call_mcp_raw(op, body, 40)
        } else {
            self.gateway.call_raw(op, body, 40)
        }
    }
    fn submit(&mut self, op: &str, args: &Value, expected: u64) -> Option<String> {
        let Ok(id) = random_id("cmd") else {
            return Some("id".into());
        };
        let body = json!({"command_id": id, "op": op, "args": args, "expected_version": expected, "wait_ms": 15000});
        self.last = Value::Null;
        let mut error: Option<String> = None;
        match self.call(op, &body) {
            Ok(Some((status, value))) => {
                if let Some(code) = value["error"]["code"].as_str() {
                    error = Some(code.to_owned());
                } else if status == 200 {
                    self.last = value;
                } else {
                    error = Some("unavailable".into());
                }
            }
            Ok(None) => error = Some("corrupt".into()),
            Err(e) => error = Some(e.to_string()),
        }
        if error.is_some()
            && let Some(value) = self.status(&id)
        {
            self.last = value;
        }
        if error.is_none() && self.disposition().is_empty() {
            let _ = await_until(45, || {
                if let Some(value) = self.status(&id) {
                    self.last = value;
                }
                !self.disposition().is_empty()
            });
        }
        if error.is_none() && self.disposition() != "locally_observed" {
            return Some(format!("disposition {}", self.disposition()));
        }
        error
    }
    fn disposition(&self) -> String {
        self.last["disposition"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    fn observed_ok(&self, error: Option<&str>) -> bool {
        error.is_none() && self.disposition() == "locally_observed"
    }
    fn native_convergence(&self) -> bool {
        calendar_fixtures::convergence(self.world, &self.observed)
    }
    fn replicas_equal(&self) -> bool {
        calendar_fixtures::replicas_equal(&self.observed)
    }
    fn control(&self, route: &str, input: &Value) -> bool {
        self.world.control(route, input).is_ok()
    }
}

fn recurrence(interval: u64, count: u64) -> Value {
    json!([{"frequency": "daily", "interval": interval, "count": count}])
}
fn unchanged(before: &[Value], now: &[Value], skip: &str) -> usize {
    now.iter()
        .filter(|r| {
            id_of(r) != skip
                && before
                    .iter()
                    .any(|b| id_of(b) == id_of(r) && b["data"] == r["data"])
        })
        .count()
}

fn protocol(
    world: &mut World,
    evidence: &mut Evidence,
    scenario: &str,
    mcp: bool,
    prefix: &str,
) -> Option<()> {
    let gateway = Client::new(world, SCOPE).ok()?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let tag = format!("[ab-test {}]", random_id("cal").ok()?);
    let mut run = Run {
        world,
        evidence,
        prefix,
        mcp,
        tag: tag.clone(),
        gateway,
        observed,
        observers: vec![],
        last: Value::Null,
        effects: BTreeMap::new(),
        latencies: BTreeMap::new(),
    };
    run.journey(scenario, &tag)
}

impl Run<'_> {
    fn journey(&mut self, scenario: &str, tag: &str) -> Option<()> {
        let title = format!("{tag} event");
        if self
            .fixture("calendar", "test-calendar", &Value::Null)
            .is_err()
        {
            return self.fail("fixture_calendar");
        }
        if !await_until(10, || !self.calendar_id().is_empty()) {
            return self.fail("calendar_visible");
        }
        self.add("calendar_visible", true);
        for client in self.endpoints() {
            match Watcher::start(&client, false) {
                Ok(watcher) => self.observers.push(watcher),
                Err(_) => return self.fail("sse_observer"),
            }
        }
        let all_day = matches!(scenario, "CAL-ALLDAY" | "CAL-ALLDAY-FUTURE");
        let (start, end) = if all_day {
            ("2026-11-01", "2026-11-03")
        } else {
            (START, END)
        };
        let mut event = json!({"title": title, "start": start, "end": end, "all_day": all_day, "time_zone": "America/New_York"});
        let recurring = matches!(
            scenario,
            "CAL-RECUR-THIS"
                | "CAL-RECUR-FUTURE"
                | "CAL-FUTURE-RULE-MISMATCH"
                | "CAL-MOVED-OCCURRENCE"
                | "CAL-MOVED-OUTSIDE-WINDOW"
                | "CAL-ALLDAY-FUTURE"
                | "CAL-SPLIT-FRAME-CAP"
        );
        if recurring {
            event["recurrence"] = recurrence(1, 3);
            if scenario == "CAL-SPLIT-FRAME-CAP" {
                event["recurrence"] = recurrence(1, 400);
            }
        }
        if scenario == "CAL-CRUD" {
            event["notes"] = json!(format!("{tag} notes"));
            event["location"] = json!(format!("{tag} location"));
            event["url"] = json!("https://example.com/calendar");
            event["availability"] = json!(1);
            event["alarms"] = json!([{"relative_offset": -600}]);
            event["calendar_id"] = json!(self.calendar_id());
            if self.submit("calendar.create_event", &event, 0).is_some() {
                return self.fail("create_event");
            }
            let seq = self.last["commit_seq"].as_u64().unwrap_or(0);
            self.add("create_event_observed", seq > 0);
        } else if self.fixture("create", "series", &event).is_err() {
            return self.fail("external_create");
        }
        let count = if scenario == "CAL-SPLIT-FRAME-CAP" {
            400
        } else if recurring {
            3
        } else {
            1
        };
        if !self.await_count(count) {
            return self.fail("initial_occurrences_visible");
        }
        self.add("initial_occurrences_visible", true);
        if scenario != "CAL-CRUD" && !self.latency(&title) {
            return self.fail("external_capture_latency");
        }
        let baseline = self.records();
        let agenda_count = if scenario == "CAL-SPLIT-FRAME-CAP" {
            4
        } else {
            count
        };
        let agenda = self.call(
            "calendar.agenda",
            &json!({"start": "2026-11-01T00:00:00Z", "end": "2026-11-04T12:00:00Z", "calendar_id": self.calendar_id()}),
        );
        let agenda_ok = matches!(&agenda, Ok(Some((200, v))) if v["error"].is_null()
            && v["records"].as_array().map_or(0, Vec::len) == agenda_count);
        self.add("agenda_returns_occurrences", agenda_ok);
        self.scenario_body(scenario, tag, &title, &event, &baseline)?;
        self.tail(scenario)
    }

    fn tail(&mut self, scenario: &str) -> Option<()> {
        if scenario != "CAL-PERMISSION-LOSS" && scenario != "CAL-SPLIT-FRAME-CAP" {
            let ok = await_until(5, || self.native_convergence());
            self.add("independent_simtruth_convergence", ok);
        }
        let ok = await_until(5, || self.replicas_equal());
        self.add("replica_convergence", ok);
        for index in 0..self.observers.len() {
            let valid = self.observers[index].valid();
            let safe = false_deletion_free(&*self.world, &self.observed, &self.observers[index]);
            self.add(&format!("contiguous_feed_{index}"), valid);
            self.add(&format!("no_false_deletion_{index}"), safe);
        }
        let keys: Vec<(String, bool)> = self
            .latencies
            .iter()
            .map(|(key, values)| (key.clone(), !values.is_empty() && p95(values) < 1000.0))
            .collect();
        for (key, ok) in keys {
            self.add(&format!("latency_{key}"), ok);
        }
        let mut clients = vec![(
            self.gateway.clone(),
            self.world
                .gateway_cli(&["token", "create", "--scope", "system:read"])
                .ok()?
                .trim()
                .to_owned(),
        )];
        for endpoint in self.observed.replicas.clone() {
            clients.push((
                endpoint.client(),
                replicas::cli(
                    &endpoint.home,
                    &["token", "create", "--scope", "system:read"],
                )
                .ok()?,
            ));
        }
        for (client, token) in clients {
            let scoped = Client::at(client.address(), &token);
            let page = scoped.call(
                "records.query",
                &json!({"collection": "calendar.event", "include_aliases": true}),
                20,
            );
            let empty = |value: &Value, key: &str| value[key].as_array().is_none_or(Vec::is_empty);
            self.add(
                "no_scope_leak_query",
                matches!(&page, Ok(Some(v)) if empty(v, "records") && empty(v, "aliases")),
            );
            let agenda = scoped.call_mcp(
                "calendar.agenda",
                &json!({"start": "2026-10-01T00:00:00Z", "end": "2026-12-01T00:00:00Z"}),
                20,
            );
            self.add(
                "no_scope_leak_mcp_agenda",
                matches!(&agenda, Ok(Some(v)) if empty(v, "records")),
            );
        }
        if scenario != "CAL-PERMISSION-LOSS" {
            let mark = self.evidence.assertions.len();
            record_integrity(self.evidence, &self.observed, None);
            for assertion in &mut self.evidence.assertions[mark..] {
                assertion.name = format!("{}{}", self.prefix, assertion.name);
            }
        }
        Some(())
    }

    fn scenario_body(
        &mut self,
        scenario: &str,
        tag: &str,
        title: &str,
        event: &Value,
        baseline: &[Value],
    ) -> Option<()> {
        match scenario {
            "CAL-CREATE-EXTERNAL" => {
                for index in 0..9 {
                    let name = format!("{tag} sample-{index}");
                    let mut copy = event.clone();
                    copy["title"] = json!(name);
                    if self
                        .fixture("create", &format!("external-{index}"), &copy)
                        .is_err()
                    {
                        return self.fail("latency_fixture");
                    }
                    if !self.latency(&name) {
                        return self.fail("latency_sample");
                    }
                }
            }
            "CAL-CRUD" => self.crud(tag)?,
            "CAL-RECUR-THIS" | "CAL-MOVED-OCCURRENCE" | "CAL-MOVED-OUTSIDE-WINDOW" => {
                self.occurrence(scenario, tag, baseline)?;
            }
            "CAL-SPLIT-FRAME-CAP" => {
                let target = baseline.get(1)?.clone();
                let error = self.submit(
                    "calendar.update_event",
                    &json!({"id": id_of(&target), "span": "future", "title": format!("{tag} oversized future")}),
                    version(&target),
                );
                let ok = self.observed_ok(error.as_deref()) && self.await_count(400);
                self.add("oversized_split_keeps_capture_progressing", ok);
                if error.is_none() {
                    let target = self.records().get(1)?.clone();
                    let error = self.submit(
                        "calendar.update_event",
                        &json!({"id": id_of(&target), "span": "this", "title": format!("{tag} continued")}),
                        version(&target),
                    );
                    let ok = self.observed_ok(error.as_deref());
                    self.add("source_captures_after_oversized_split", ok);
                }
            }
            "CAL-ALLDAY-FUTURE" => self.allday_future(baseline)?,
            "CAL-FUTURE-RULE-MISMATCH" => self.rule_mismatch(tag, baseline)?,
            "CAL-RECUR-FUTURE" => self.recur_future(tag, title, baseline)?,
            "CAL-RECUR-ALL-REJECT" => {
                let target = baseline.first()?.clone();
                let error = self.submit(
                    "calendar.update_event",
                    &json!({"id": id_of(&target), "span": "all", "title": format!("{tag} forbidden")}),
                    version(&target),
                );
                let rejected = error.as_deref() == Some("unsupported")
                    && self.last["command"]["command_id"]
                        .as_str()
                        .is_none_or(str::is_empty);
                self.add("span_all_rejected_before_dispatch", rejected);
                let same = baseline == self.records().as_slice();
                self.add("rejected_span_preserves_events", same);
            }
            "CAL-ALLDAY" => {
                let value = baseline.first()?;
                let ok = value["data"]["all_day"] == true
                    && value["data"]["start"] == "2026-11-01"
                    && value["data"]["end"] == "2026-11-03";
                self.add("floating_dates_preserved", ok);
            }
            "CAL-ID-CHURN" => {
                if self.fixture("churn", "", &Value::Null).is_err() {
                    return self.fail("churn_fixture");
                }
                let first = baseline.first()?.clone();
                let ok = await_until(3, || {
                    let now = self.records();
                    now.len() == 1
                        && id_of(&now[0]) == id_of(&first)
                        && version(&now[0]) == version(&first)
                });
                self.add("stable_external_alias_identity", ok);
            }
            "CAL-WINDOW-SLIDE" => self.window_slide(tag)?,
            "CAL-READONLY" => self.readonly(tag)?,
            "CAL-PERMISSION-LOSS" => self.permission_loss(baseline)?,
            other => {
                return self.fail(&format!("scenario_not_ported_{other}"));
            }
        }
        Some(())
    }

    fn split(&self) -> Split {
        Split::start(self.endpoints(), &self.tag)
    }

    fn crud(&mut self, tag: &str) -> Option<()> {
        let available = self.records();
        if available.len() != 1 {
            return self.fail("created_record_available_for_update");
        }
        let current = available[0].clone();
        let error = self.submit(
            "calendar.update_event",
            &json!({"id": id_of(&current), "span": "this", "title": format!("{tag} updated"),
                "notes": format!("{tag} changed notes"), "location": format!("{tag} changed location"),
                "url": "https://example.com/updated", "availability": 2,
                "alarms": [{"relative_offset": -300}]}),
            version(&current),
        );
        if error.is_some() {
            return self.fail("update_event");
        }
        if !self.await_count(1) {
            return self.fail("updated_visible");
        }
        let available = self.records();
        if available.len() != 1 {
            return self.fail("updated_record_available");
        }
        let updated = available[0].clone();
        self.add(
            "update_preserves_bridge_id",
            id_of(&updated) == id_of(&current),
        );
        let fields = &updated["data"];
        let alarms = fields["alarms"].as_array().cloned().unwrap_or_default();
        let alarm_ok = alarms.len() == 1 && alarms[0]["relative_offset"].as_f64() == Some(-300.0);
        let ok = fields["notes"] == format!("{tag} changed notes")
            && fields["location"] == format!("{tag} changed location")
            && fields["url"] == "https://example.com/updated"
            && fields["availability"].as_f64() == Some(2.0)
            && alarm_ok;
        self.add("event_fields_and_alarm_updated", ok);
        let current = updated;
        let stale = self.call(
            "calendar.update_event",
            &json!({"op": "calendar.update_event",
                "args": {"id": id_of(&current), "span": "this", "title": format!("{tag} stale")},
                "expected_version": version(&current).saturating_sub(1), "wait_ms": 15000}),
        );
        let conflict = matches!(&stale, Ok(Some((_, v)))
            if v["error"]["code"] == "conflict" && v["error"]["current"]["id"] == current["id"]);
        self.add("stale_version_rejected", conflict);
        if self
            .submit(
                "calendar.delete_event",
                &json!({"id": id_of(&current), "span": "this"}),
                version(&current),
            )
            .is_some()
        {
            return self.fail("delete_event");
        }
        let gone = self.await_count(0);
        self.add("delete_absence_visible", gone);
        let args = json!({"calendar_id": self.calendar_id(), "title": format!("{tag} created series"),
            "start": START, "end": END, "time_zone": "America/New_York", "recurrence": recurrence(1, 3)});
        if self.submit("calendar.create_event", &args, 0).is_some() {
            return self.fail("create_recurring_command");
        }
        if !self.await_count(3) {
            return self.fail("created_series_occurrences");
        }
        let available = self.records();
        if available.len() != 3 {
            return self.fail("created_series_records_available");
        }
        let created = available[0].clone();
        if self
            .submit(
                "calendar.delete_event",
                &json!({"id": id_of(&created), "span": "future"}),
                version(&created),
            )
            .is_some()
        {
            return self.fail("delete_created_series");
        }
        let gone = self.await_count(0);
        self.add("created_series_deleted_with_future_span", gone);
        if self
            .submit(
                "calendar.create_calendar",
                &json!({"title": format!("Bridge Test {tag}")}),
                0,
            )
            .is_some()
        {
            return self.fail("create_calendar");
        }
        let seq = self.last["commit_seq"].as_u64().unwrap_or(0);
        self.add("create_calendar_observed", seq > 0);
        Some(())
    }

    fn occurrence(&mut self, scenario: &str, tag: &str, baseline: &[Value]) -> Option<()> {
        let target = baseline.get(1)?.clone();
        let moved = scenario != "CAL-RECUR-THIS";
        let exception = format!("{tag} exception");
        let ok = if moved {
            let aliases = gateway_aliases(&self.gateway).ok()?;
            let prefix = "calendar/calendar.event/";
            let native = aliases
                .iter()
                .find(|(alias, id)| **id == id_of(&target) && alias.starts_with(prefix))
                .map(|(alias, _)| alias.trim_start_matches(prefix).to_owned())
                .unwrap_or_default();
            let start = if scenario == "CAL-MOVED-OUTSIDE-WINDOW" {
                civil_now_plus_years(3)?
            } else {
                "2026-11-02T08:30:00Z".to_owned()
            };
            let mut data = json!({"index": 1, "new_title": exception, "start": start});
            if scenario == "CAL-MOVED-OCCURRENCE" {
                data["end"] = json!("2026-11-02T10:30:00Z");
            }
            self.fixture("move", &native, &data).is_ok()
        } else {
            self.submit(
                "calendar.update_event",
                &json!({"id": id_of(&target), "span": "this", "title": exception}),
                version(&target),
            )
            .is_none()
        };
        if !ok {
            return self.fail("update_occurrence");
        }
        if scenario == "CAL-MOVED-OUTSIDE-WINDOW" {
            let evicted = self.await_count(2);
            self.add("moved_outside_window_evicted", evicted);
            let never = await_until(3, || {
                self.observers.iter().all(|observer| {
                    let mut evicted = false;
                    let mut deleted = false;
                    for change in observer.changes() {
                        if change["record"]["id"] == target["id"] {
                            evicted = evicted
                                || (change["op"] == "evict"
                                    && change["record"]["tombstone_reason"] == "window_evicted");
                            deleted = deleted || change["op"] == "delete";
                        }
                    }
                    evicted && !deleted
                })
            });
            self.add("moved_outside_window_never_deleted", never);
            let now = self.records();
            let siblings = now
                .iter()
                .map(|r| {
                    baseline
                        .iter()
                        .filter(|b| {
                            id_of(b) != id_of(&target)
                                && id_of(b) == id_of(r)
                                && b["data"] == r["data"]
                        })
                        .count()
                })
                .sum::<usize>();
            self.add("outside_window_move_preserves_siblings", siblings == 2);
            return Some(());
        }
        if !self.await_count(3) {
            return self.fail("three_occurrences_after_exception");
        }
        let visible = await_until(5, || {
            self.records()
                .iter()
                .find(|r| r["id"] == target["id"])
                .is_some_and(|r| text(r, "title") == exception && r["data"]["is_exception"] == true)
        });
        self.add("exception_capture_visible", visible);
        let after = self.records();
        let mut kept = false;
        for r in &after {
            if r["id"] == target["id"] {
                self.add(
                    "exception_has_scoped_native_recurrence",
                    !r["data"]["recurrence"].is_null(),
                );
                kept = r["data"]["occurrence_date"] == target["data"]["occurrence_date"]
                    && r["data"]["is_exception"] == true
                    && text(r, "title") == exception;
                if scenario == "CAL-MOVED-OCCURRENCE" {
                    let ok = same_time(&r["data"]["start"], &json!("2026-11-02T08:30:00Z"))
                        && same_time(&r["data"]["end"], &json!("2026-11-02T10:30:00Z"));
                    self.add("moved_start_and_end", ok);
                }
            }
        }
        let same = unchanged(baseline, &after, &id_of(&target));
        self.add("original_occurrence_identity_preserved", kept);
        self.add("other_occurrences_unchanged", same == 2);
        if scenario == "CAL-RECUR-THIS" {
            let deleted = baseline.get(2)?.clone();
            let error = self.submit(
                "calendar.delete_event",
                &json!({"id": id_of(&deleted), "span": "this"}),
                version(&deleted),
            );
            let seq = self.last["commit_seq"].as_u64().unwrap_or(0);
            let ok = error.is_none() && self.disposition() == "locally_observed" && seq > 0;
            self.add("delete_one_occurrence_is_observed", ok);
            let survives = self.await_count(2);
            self.add("delete_one_occurrence_keeps_surviving_series", survives);
            let remaining = self.records();
            let preserved = remaining.len() == 2
                && remaining.iter().all(|r| {
                    r["id"] != deleted["id"]
                        && (r["id"] == baseline[0]["id"] || r["id"] == baseline[1]["id"])
                });
            self.add(
                "delete_one_occurrence_preserves_other_identities",
                preserved,
            );
        }
        Some(())
    }

    fn allday_future(&mut self, baseline: &[Value]) -> Option<()> {
        let split = self.split();
        let target = baseline.get(1)?.clone();
        if self
            .submit(
                "calendar.update_event",
                &json!({"id": id_of(&target), "span": "future", "start": "2026-11-04", "end": "2026-11-06"}),
                version(&target),
            )
            .is_some()
        {
            return self.fail("all_day_future_update");
        }
        let offsets = await_until(3, || {
            let after = self.records();
            after.len() == 3
                && after[0]["data"]["start"] == "2026-11-01"
                && after[1]["data"]["start"] == "2026-11-04"
                && after[2]["data"]["start"] == "2026-11-05"
                && after[2]["data"]["end"] == "2026-11-07"
        });
        self.add("all_day_future_civil_offsets", offsets);
        let after = self.records();
        let mut identities = after.len() == 3;
        if identities {
            identities = after[0]["id"] == baseline[0]["id"]
                && after[1]["data"]["series_id"] != baseline[1]["data"]["series_id"]
                && after[1]["id"] != baseline[1]["id"]
                && after[2]["id"] != baseline[2]["id"];
            for record in &after[1..] {
                identities =
                    identities && record["data"]["occurrence_date"] == record["data"]["start"];
            }
        }
        self.add("future_split_replaces_occurrence_identities", identities);
        let atomic = split.finish();
        self.add("future_split_atomic_on_gateway_and_replica", atomic);
        Some(())
    }

    fn rule_mismatch(&mut self, tag: &str, baseline: &[Value]) -> Option<()> {
        let armed = self.control("/fault", &json!({"name": "calendar-rule-mismatch"}));
        self.add("mismatch_fault_set", armed);
        let target = baseline.get(1)?.clone();
        let error = self.submit(
            "calendar.update_event",
            &json!({"id": id_of(&target), "span": "future", "title": format!("{tag} native mismatch"),
                "recurrence": recurrence(2, 2)}),
            version(&target),
        );
        let evidence = self.last["evidence"].clone();
        let decoded = evidence.is_object();
        let reason = evidence["reason"] == "native_postcondition_mismatch";
        self.add(
            "mismatch_is_indeterminate_after_effect",
            error.is_some() && self.disposition() == "indeterminate" && reason,
        );
        let rules = evidence["native_result"]["recurrence"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let actual = decoded
            && evidence["observed"] != true
            && evidence["native_result"]["title"] == format!("{tag} native mismatch")
            && rules.len() == 1
            && rules[0]["count"].as_u64() == Some(1)
            && rules[0]["interval"].as_u64() == Some(2);
        self.add("mismatch_reports_actual_native_rule", actual);
        let command = self.last["command"]["command_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let effects = self.world.control("/effects", &Value::Null).ok();
        let once = effects.is_some_and(|e| e[command.as_str()].as_u64() == Some(1));
        self.add("mismatch_never_retries_or_writes_twice", once);
        let none = self.last["commit_seq"].as_u64().unwrap_or(0) == 0
            && self.last["observed_record_ids"]
                .as_array()
                .is_none_or(Vec::is_empty);
        self.add("mismatch_never_claims_observed", none);
        Some(())
    }

    fn recur_future(&mut self, tag: &str, title: &str, baseline: &[Value]) -> Option<()> {
        let exception = baseline.get(2)?.clone();
        let preserved_title = format!("{tag} preserved exception");
        if self
            .submit(
                "calendar.update_event",
                &json!({"id": id_of(&exception), "span": "this", "title": preserved_title}),
                version(&exception),
            )
            .is_some()
        {
            return self.fail("seed_exception");
        }
        let seeded = await_until(3, || {
            self.records()
                .iter()
                .find(|r| r["id"] == exception["id"])
                .is_some_and(|r| text(r, "title") == preserved_title)
        });
        if !seeded {
            return self.fail("exception_visible");
        }
        let split = self.split();
        let target = self.records().get(1)?.clone();
        if self
            .submit(
                "calendar.update_event",
                &json!({"id": id_of(&target), "span": "future", "title": format!("{tag} future"),
                    "recurrence": recurrence(2, 2)}),
                version(&target),
            )
            .is_some()
        {
            return self.fail("future_update");
        }
        let regenerated = self.await_count(4);
        self.add("new_rule_regenerates_occurrences", regenerated);
        let after = self.records();
        let mut preserved = false;
        let mut generated = false;
        let mut identities = after.len() == 4;
        for r in &after {
            let d = &r["data"];
            preserved = preserved
                || (text(r, "title") == preserved_title
                    && d["is_exception"] == false
                    && d["series_id"].is_null()
                    && d["occurrence_date"].is_null()
                    && r["id"] != exception["id"]
                    && same_time(&d["start"], &exception["data"]["start"]));
            if text(r, "title") == title {
                identities = identities && r["id"] == baseline[0]["id"];
            } else {
                for old in &baseline[1..] {
                    identities = identities && r["id"] != old["id"];
                }
            }
            generated = generated
                || (text(r, "title") == format!("{tag} future")
                    && same_time(&d["start"], &json!("2026-11-04T06:30:00Z")));
        }
        self.add(
            "future_split_preserves_exception_as_new_standalone",
            preserved,
        );
        self.add("future_split_replaces_occurrence_identities", identities);
        self.add("future_rule_changes_dates", generated);
        let series: BTreeSet<String> = after
            .iter()
            .filter_map(|r| r["data"]["series_id"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        self.add("future_span_splits_series", series.len() == 2);
        let atomic = split.finish();
        self.add("future_split_atomic_on_gateway_and_replica", atomic);
        for candidate in &after {
            if text(candidate, "title") != format!("{tag} future") {
                continue;
            }
            if self
                .submit(
                    "calendar.delete_event",
                    &json!({"id": id_of(candidate), "span": "future"}),
                    version(candidate),
                )
                .is_some()
            {
                return self.fail("delete_future");
            }
            let counted = self.await_count(2);
            let now = self.records();
            let ok = counted
                && now.len() == 2
                && text(&now[0], "title") == title
                && text(&now[1], "title") == preserved_title;
            self.add("delete_future_preserves_past_and_standalone", ok);
            break;
        }
        Some(())
    }

    fn window_slide(&mut self, tag: &str) -> Option<()> {
        let old = civil_midnight(-730).ok()?;
        let future = civil_midnight(730)
            .ok()?
            .checked_add(Duration::from_secs(3600))
            .ok()?;
        for (index, date) in [old, future].into_iter().enumerate() {
            let end = date.checked_add(Duration::from_secs(3600)).ok()?;
            let value = json!({"title": format!("{tag} boundary-{index}"), "start": date.to_string(), "end": end.to_string()});
            if self
                .fixture("create", &format!("boundary-{index}"), &value)
                .is_err()
            {
                return self.fail("window_fixture");
            }
        }
        if !self.await_count(2) {
            return self.fail("initial_window_boundaries");
        }
        let next = civil_midnight(1).ok()?;
        if !self.control(
            "/calendar",
            &json!({"action": "clock", "now": next.to_string()}),
        ) {
            return self.fail("slide_clock");
        }
        let member = self.await_count(2);
        self.add("sliding_window_membership", member);
        let boundary = format!("{tag} boundary-0");
        let inspect = |observers: &[Watcher]| {
            let (mut evicted, mut removed, mut settled) = (false, false, true);
            for observer in observers {
                let mut done = false;
                for change in observer.changes() {
                    if change["record"]["data"]["title"] == boundary {
                        let evict = change["op"] == "evict"
                            && change["record"]["tombstone_reason"] == "window_evicted";
                        evicted = evicted || evict;
                        removed = removed || change["op"] == "delete";
                        done = done || evict || change["op"] == "delete";
                    }
                }
                settled = settled && done;
            }
            (evicted, removed, settled)
        };
        let _ = await_until(5, || inspect(&self.observers).2);
        let (evicted, removed, _) = inspect(&self.observers);
        self.add("window_eviction_is_not_deletion", evicted && !removed);
        Some(())
    }

    fn readonly(&mut self, tag: &str) -> Option<()> {
        let data = json!({"title": format!("{tag} readonly"), "writable": false, "type": "subscribed", "account": "test"});
        if self.fixture("calendar", "readonly", &data).is_err() {
            return self.fail("readonly_fixture");
        }
        let mut readonly = Value::Null;
        let found = await_until(5, || {
            let all = read_all(&self.gateway, false, false)
                .map(|read| read.records)
                .unwrap_or_default();
            all.iter()
                .find(|r| {
                    r["collection"] == "calendar.calendar"
                        && r["data"]["writable"] == false
                        && r["data"]["type"] == "subscribed"
                })
                .is_some_and(|r| {
                    readonly = r.clone();
                    true
                })
        });
        if !found {
            return self.fail("readonly_calendar_visible");
        }
        let args = json!({"calendar_id": id_of(&readonly), "title": format!("{tag} rejected"), "start": START, "end": END});
        let error = self.submit("calendar.create_event", &args, 0);
        let dispatched = self.last["history"]
            .as_array()
            .is_some_and(|h| h.iter().any(|t| t["state"] == "dispatched"));
        let rejected = error.as_deref() == Some("unsupported")
            && self.disposition() == "rejected_before_dispatch"
            && !dispatched;
        self.add("readonly_rejected_before_dispatch", rejected);
        let none = self.records().len() == 1;
        self.add("readonly_has_no_effect", none);
        Some(())
    }

    fn permission_loss(&mut self, baseline: &[Value]) -> Option<()> {
        if !self.control(
            "/source-state",
            &json!({"source": "calendar", "state": "permission_missing"}),
        ) {
            return self.fail("permission_fault");
        }
        if self.fixture("delete", "series", &Value::Null).is_err() {
            return self.fail("external_delete_denied");
        }
        thread::sleep(Duration::from_millis(150));
        let same = baseline == self.records().as_slice();
        self.add("permission_loss_preserves_records", same);
        let all = read_all(&self.gateway, false, false)
            .map(|read| read.records)
            .unwrap_or_default();
        let missing = all.iter().any(|r| {
            r["collection"] == "system.source"
                && r["data"]["source"] == "calendar"
                && r["data"]["state"] == "permission_missing"
        });
        self.add("permission_missing_published", missing);
        if self.world.restart_peer().is_err() {
            return self.fail("restart_while_permission_missing");
        }
        thread::sleep(Duration::from_millis(200));
        let same = baseline == self.records().as_slice();
        self.add("permission_loss_survives_restart", same);
        Some(())
    }
}

fn civil_now_plus_years(years: i16) -> Option<String> {
    let now = Timestamp::now().to_zoned(TimeZone::UTC);
    now.checked_add(Span::new().years(i64::from(years)))
        .ok()
        .map(|z| z.timestamp().to_string())
}
