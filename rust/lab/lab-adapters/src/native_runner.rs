use crate::{
    assertions::{Visibility, await_until, gateway_aliases, record_multiset, truth_records},
    http::{Client, random_id},
    journey::{GATEWAY_SCOPE, all_endpoints, gateway_client, records},
    replicas::{self, Endpoint},
    watch::{Cursor, Watcher},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 7] = [
    "REM-CRUD",
    "REM-CREATE-EXTERNAL",
    "REM-LIST-MANAGE",
    "REM-LIST-CREATE",
    "REM-LIST-LOST-ACK",
    "REM-RECURRING",
    "REM-PERMISSION-LOSS",
];
const LIST: &str = "Bridge Test";
const REMINDER: &str = "reminders.reminder";
const REMINDER_LIST: &str = "reminders.list";

static EPOCH: OnceLock<Instant> = OnceLock::new();

#[must_use]
pub fn instant_ns(at: Instant) -> u64 {
    let epoch = *EPOCH.get_or_init(Instant::now);
    u64::try_from(at.saturating_duration_since(epoch).as_nanos()).unwrap_or(u64::MAX)
}
#[must_use]
pub fn uptime_ns() -> u64 {
    instant_ns(Instant::now()) + 1
}
fn millis_between(from_ns: u64, to_ns: u64) -> f64 {
    Duration::from_nanos(to_ns.saturating_sub(from_ns)).as_secs_f64() * 1000.0
}

pub fn records_at(client: &Client) -> Result<(Cursor, Vec<Value>)> {
    let mut out = vec![];
    let mut after = String::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let page = client
            .call(
                "records.query",
                &json!({"collection":"","limit":1000,"after":after}),
                20,
            )?
            .ok_or_else(|| Error::Invalid("records.query rejected".into()))?;
        let current = Cursor::from_value(&page["cursor"]);
        match &cursor {
            None => cursor = Some(current),
            Some(first) if *first != current => {
                return Err(Error::Invalid("snapshot changed during pagination".into()));
            }
            Some(_) => {}
        }
        out.extend(page["records"].as_array().cloned().unwrap_or_default());
        match page["next"].as_str() {
            Some(next) if !next.is_empty() => next.clone_into(&mut after),
            _ => return Ok((cursor.unwrap_or(Cursor::from_value(&Value::Null)), out)),
        }
    }
}
#[must_use]
pub fn without_commands(records: Vec<Value>) -> Vec<Value> {
    records
        .into_iter()
        .filter(|r| r["collection"] != "system.command")
        .collect()
}
pub fn replicas_equal_gateway(gateway: &Client, replicas: &[Endpoint]) -> bool {
    let Ok((head, records)) = records_at(gateway) else {
        return false;
    };
    let records = without_commands(records);
    replicas.iter().all(|endpoint| {
        records_at(&endpoint.client())
            .is_ok_and(|(applied, replica)| applied == head && without_commands(replica) == records)
    })
}

fn title_of(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}
fn find(client: &Client, collection: &str, title: &str) -> Option<Value> {
    records(client)
        .ok()?
        .1
        .into_iter()
        .find(|r| r["collection"] == collection && title_of(r) == title)
}
fn version_of(record: &Value) -> u64 {
    record["version"].as_u64().unwrap_or(0)
}

struct Out<'a> {
    evidence: &'a mut Evidence,
    prefix: &'a str,
}
impl Out<'_> {
    fn add(&mut self, name: &str, passed: bool) {
        self.evidence
            .check(&format!("{}{name}", self.prefix), passed, "");
    }
    fn add_detail(&mut self, name: &str, passed: bool, detail: &str) {
        self.evidence
            .check(&format!("{}{name}", self.prefix), passed, detail);
    }
    fn fail(&mut self, name: &str) -> bool {
        self.add(name, false);
        false
    }
}

struct Native<'a> {
    world: &'a World,
    gateway: Client,
    replicas: Vec<Endpoint>,
    mcp: bool,
}
impl Native<'_> {
    fn endpoints(&self) -> Vec<Client> {
        std::iter::once(self.gateway.clone())
            .chain(self.replicas.iter().map(Endpoint::client))
            .collect()
    }
    fn ready(&self) -> bool {
        records(&self.gateway).is_ok_and(|(_, all)| {
            all.iter().any(|r| {
                r["collection"] == "system.source"
                    && r["data"]["source"] == "reminders"
                    && r["data"]["state"] == "ready"
            })
        })
    }
    fn fixture_create(&self, title: &str, recurring: bool) -> Result<u64> {
        let effect = uptime_ns();
        let mut data = json!({
            "title": title,
            "completed": false,
            "list_id": {"$ref": {"collection": REMINDER_LIST, "native_key": "list"}}
        });
        if recurring {
            data["recurrence"] = json!("daily");
            data["coverage"] = json!("next_incomplete_instance_only");
        }
        self.world
            .control("/external-change", &json!({"key": title, "data": data}))?;
        Ok(effect)
    }
    fn submit(&self, op: &str, args: &Value, version: u64) -> Result<Option<Value>> {
        let mut command = json!({
            "command_id": random_id("cmd")?,
            "op": op,
            "args": args,
            "wait_ms": 15000
        });
        if version > 0 {
            command["expected_version"] = json!(version);
        }
        let reply = if self.mcp {
            self.gateway.call_mcp_raw(op, &command, 40)?
        } else {
            self.gateway.call_raw(op, &command, 40)?
        };
        let value = reply.map_or(Value::Null, |(_, value)| value);
        Ok(
            (value["error"].is_null() && value["disposition"] == "locally_observed")
                .then_some(value),
        )
    }
    fn visible_everywhere(&self, title: &str) -> bool {
        self.replicas
            .iter()
            .all(|e| find(&e.client(), REMINDER, title).is_some())
    }
    fn simulated_truth_converged(&self) -> bool {
        let (Ok(truth), Ok(aliases), Ok((_, gateway))) = (
            self.world.control("/inspect", &Value::Null),
            gateway_aliases(&self.gateway),
            records(&self.gateway),
        ) else {
            return false;
        };
        let visibility = Visibility::from_scope(GATEWAY_SCOPE);
        let Ok(expected) = truth_records(&truth, &aliases, &visibility) else {
            return false;
        };
        let domain: Vec<Value> = gateway
            .iter()
            .filter(|r| {
                !r["collection"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("system.")
            })
            .cloned()
            .collect();
        if record_multiset(&domain, false) != record_multiset(&expected, false) {
            return false;
        }
        let gateway = without_commands(gateway);
        self.replicas.iter().all(|e| {
            records(&e.client()).is_ok_and(|(_, replica)| without_commands(replica) == gateway)
        })
    }
    fn epilogue(&self, id: &str, out: &mut Out) -> Result<()> {
        if id != "REM-PERMISSION-LOSS" {
            out.add(
                "independent_simtruth_convergence",
                await_until(5, || self.simulated_truth_converged()),
            );
        }
        out.add(
            "replica_convergence",
            await_until(5, || replicas_equal_gateway(&self.gateway, &self.replicas)),
        );
        for (_, client, token) in all_endpoints(self.world)? {
            let limited = Client::at(client.address(), &token);
            let page = limited.call("records.query", &json!({"collection": REMINDER}), 20)?;
            out.add(
                "no_scope_leak",
                page.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty)),
            );
        }
        Ok(())
    }
    fn protocol(&self, id: &str, out: &mut Out) -> Result<bool> {
        if !await_until(15, || self.ready()) {
            return Ok(out.fail("reminders_ready"));
        }
        out.add("reminders_ready", true);
        let tag = format!("[ab-test {}]", random_id("m1")?);
        let title = format!("{tag} reminder");
        let proceed = match id {
            "REM-CREATE-EXTERNAL" => self.create_external(&title, out),
            "REM-LIST-MANAGE" => self.list_manage(&tag, out)?,
            "REM-LIST-CREATE" | "REM-LIST-LOST-ACK" => self.list_create(id, out)?,
            "REM-PERMISSION-LOSS" => self.permission_loss(&title, out)?,
            _ => self.reminder_flow(id, &tag, title, out)?,
        };
        if !proceed {
            return Ok(false);
        }
        self.epilogue(id, out)?;
        Ok(true)
    }

    fn create_external(&self, title: &str, out: &mut Out) -> bool {
        let mut observers = vec![];
        for client in self.endpoints() {
            match Watcher::start(&client, false) {
                Ok(watcher) => observers.push(watcher),
                Err(_) => return out.fail("external_visibility_observer"),
            }
        }
        let Ok(effect) = self.fixture_create(title, false) else {
            return out.fail("fixture_create");
        };
        if effect == 0 {
            return out.fail("native_monotonic_timestamp");
        }
        let seen_all = |observers: &[Watcher]| {
            observers
                .iter()
                .all(|o| o.first_seen_title(REMINDER, title).is_some())
        };
        if !await_until(3, || seen_all(&observers)) {
            return out.fail("replica_visible");
        }
        for (index, observer) in observers.iter().enumerate() {
            let seen = observer
                .first_seen_title(REMINDER, title)
                .map_or(0, instant_ns);
            if seen < effect {
                return out.fail("monotonic_clock_agreement");
            }
            let value = millis_between(effect, seen);
            let assertion = if index == 0 {
                "gateway_visible_under_one_second"
            } else {
                "replica_visible_under_one_second"
            };
            out.add_detail(assertion, value < 1000.0, &format!("{value:.1} ms"));
            out.add("contiguous_external_visibility_feed", observer.valid());
        }
        out.add(
            "replica_read_after_first_visibility",
            await_until(3, || self.visible_everywhere(title)),
        );
        let poll = millis_between(effect, uptime_ns());
        out.add_detail(
            "replica_poll_completed_under_one_second",
            poll < 1000.0,
            &format!("{poll:.1} ms"),
        );
        true
    }

    fn list_manage(&self, tag: &str, out: &mut Out) -> Result<bool> {
        let name = format!("Bridge Test {tag}");
        if self
            .submit("reminders.create_list", &json!({"title": name}), 0)?
            .is_none()
        {
            return Ok(out.fail("create_owned_list"));
        }
        let mut owned = None;
        if !await_until(3, || {
            owned = find(&self.gateway, REMINDER_LIST, &name);
            owned.is_some()
        }) {
            return Ok(out.fail("owned_list_capture"));
        }
        let owned = owned.unwrap_or(Value::Null);
        let renamed = format!("{name} renamed");
        let version = version_of(&owned);
        let stale = self.submit(
            "reminders.rename_list",
            &json!({"id": owned["id"], "title": renamed}),
            version + 1,
        )?;
        out.add("stale_rename_rejected", stale.is_none());
        if self
            .submit(
                "reminders.rename_list",
                &json!({"id": owned["id"], "title": renamed}),
                version,
            )?
            .is_none()
        {
            return Ok(out.fail("rename_owned_list"));
        }
        let mut current = Value::Null;
        out.add(
            "rename_readback",
            await_until(3, || {
                find(&self.gateway, REMINDER_LIST, &renamed).is_some_and(|r| {
                    current = r;
                    true
                })
            }),
        );
        let version = version_of(&current);
        let stale = self.submit(
            "reminders.delete_list",
            &json!({"id": owned["id"]}),
            version + 1,
        )?;
        out.add("stale_delete_rejected", stale.is_none());
        if self
            .submit(
                "reminders.delete_list",
                &json!({"id": owned["id"]}),
                version,
            )?
            .is_none()
        {
            return Ok(out.fail("delete_owned_list"));
        }
        out.add(
            "delete_readback",
            await_until(3, || find(&self.gateway, REMINDER_LIST, &renamed).is_none()),
        );
        Ok(true)
    }

    fn list_create(&self, id: &str, out: &mut Out) -> Result<bool> {
        let lost = id == "REM-LIST-LOST-ACK";
        if lost {
            let armed = self
                .world
                .gateway_home
                .join("helper")
                .join("lab-pause-outcome");
            let armed_ok = armed
                .parent()
                .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
                && std::fs::write(&armed, "armed").is_ok();
            if !armed_ok {
                return Ok(out.fail("arm_lost_list_outcome"));
            }
        }
        let submitted = self.submit("reminders.create_list", &json!({"title": LIST}), 0)?;
        if lost {
            out.add("list_response_ambiguous", submitted.is_none());
            let paused = self
                .world
                .gateway_home
                .join("helper")
                .join("lab-paused-outcome")
                .exists();
            out.evidence.unverified(
                "native_list_effect_before_lost_outcome",
                &format!("paused outcome marker present: {paused}; native list inventory needs AppleTruth at the real level"),
            );
        } else if submitted.is_none() {
            return Ok(out.fail("rest_create_list"));
        }
        out.add(
            "list_visible",
            await_until(3, || find(&self.gateway, REMINDER_LIST, LIST).is_some()),
        );
        Ok(true)
    }

    fn permission_loss(&self, title: &str, out: &mut Out) -> Result<bool> {
        if self.fixture_create(title, false).is_err() {
            return Ok(out.fail("fixture_create"));
        }
        if self.world.control("/flush", &json!({})).is_err() {
            return Ok(out.fail("capture_committed"));
        }
        let before = records(&self.gateway).map(|r| r.1).unwrap_or_default();
        if self
            .world
            .control(
                "/source-state",
                &json!({"source": "reminders", "state": "permission_missing"}),
            )
            .is_err()
        {
            return Ok(out.fail("permission_loss_fault"));
        }
        out.add(
            "permission_missing_reported",
            await_until(3, || !self.ready()),
        );
        if self
            .world
            .control("/external-change", &json!({"key": title, "delete": true}))
            .is_err()
        {
            return Ok(out.fail("external_delete_while_denied"));
        }
        std::thread::sleep(Duration::from_millis(150));
        let after = records(&self.gateway);
        out.add(
            "no_false_deletion",
            after.is_ok_and(|(_, after)| {
                record_multiset(&before, false) == record_multiset(&after, false)
            }),
        );
        let admin = Client::new(self.world, "admin")?;
        let message = self
            .world
            .control(
                "/external-change",
                &json!({
                    "key": "permission-isolated-message",
                    "collection": "messages.message",
                    "data": {
                        "title": "[ab-test] permission-isolated-message",
                        "text": "restrictedneedle",
                        "optional_future": {"counter": 9_007_199_254_740_993_u64}
                    }
                }),
            )
            .is_ok();
        if !message {
            return Ok(out.fail("other_source_during_permission_loss"));
        }
        out.add(
            "reminders_permission_loss_does_not_suppress_messages",
            await_until(3, || {
                admin
                    .call(
                        "records.query",
                        &json!({"collection": "messages.message"}),
                        20,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 1))
            }),
        );
        Ok(true)
    }

    fn reminder_flow(&self, id: &str, tag: &str, mut title: String, out: &mut Out) -> Result<bool> {
        if id == "REM-CRUD" {
            let Some(list) = ({
                let mut list = None;
                await_until(3, || {
                    list = find(&self.gateway, REMINDER_LIST, LIST);
                    list.is_some()
                });
                list
            }) else {
                return Ok(out.fail("list_visible"));
            };
            if self
                .submit(
                    "reminders.create",
                    &json!({"list_id": list["id"], "title": title}),
                    0,
                )?
                .is_none()
            {
                return Ok(out.fail("rest_reminders.create"));
            }
            out.add("rest_reminders.create", true);
        } else if self.fixture_create(&title, id == "REM-RECURRING").is_err() {
            return Ok(out.fail("fixture_create"));
        }
        let effect_done = Instant::now();
        if !await_until(3, || self.visible_everywhere(&title)) {
            return Ok(out.fail("replica_visible"));
        }
        let elapsed = effect_done.elapsed().as_secs_f64() * 1000.0;
        out.add_detail(
            "replica_visible_under_one_second",
            elapsed < 1000.0,
            &format!("{elapsed:.1} ms"),
        );
        if id == "REM-CRUD" && !self.crud(tag, &mut title, out)? {
            return Ok(false);
        }
        if id == "REM-RECURRING" && !self.recurring(&title, out) {
            return Ok(false);
        }
        Ok(true)
    }

    fn crud(&self, tag: &str, title: &mut String, out: &mut Out) -> Result<bool> {
        for operation in [
            "reminders.update",
            "reminders.complete",
            "reminders.uncomplete",
            "reminders.delete",
        ] {
            let mut record = None;
            if !await_until(3, || {
                record = find(&self.gateway, REMINDER, title);
                record.is_some()
            }) {
                return Ok(out.fail("command_target_visible"));
            }
            let record = record.unwrap_or(Value::Null);
            let mut args = json!({"id": record["id"]});
            if operation == "reminders.update" {
                *title = format!("{tag} updated");
                args["title"] = json!(title);
            }
            if self
                .submit(operation, &args, version_of(&record))?
                .is_none()
            {
                return Ok(out.fail(&format!("rest_{operation}")));
            }
            out.add(&format!("rest_{operation}"), true);
            let completed = operation == "reminders.complete";
            let visible = await_until(3, || {
                self.endpoints().iter().all(|client| {
                    let current = find(client, REMINDER, title);
                    if operation == "reminders.delete" {
                        current.is_none()
                    } else {
                        current.is_some_and(|r| r["data"]["completed"] == completed)
                    }
                })
            });
            if !visible {
                return Ok(out.fail(&format!("read_your_write_{operation}")));
            }
        }
        Ok(true)
    }

    fn recurring(&self, title: &str, out: &mut Out) -> bool {
        let Some(original) = find(&self.gateway, REMINDER, title) else {
            return out.fail("recurring_original_visible");
        };
        if self
            .world
            .control("/complete-recurring", &json!({"key": title}))
            .is_err()
        {
            return out.fail("complete_recurring_fixture");
        }
        let advanced = await_until(5, || {
            records(&self.gateway).is_ok_and(|(_, all)| {
                all.iter().any(|r| {
                    r["collection"] == REMINDER
                        && title_of(r) == title
                        && r["data"]["completed"] == false
                        && (r["id"] != original["id"]
                            || r["data"]["due"] != original["data"]["due"])
                })
            })
        });
        out.add("next_incomplete_recurring_instance", advanced);
        true
    }
}

pub fn register(registry: &mut lab_runner::Registry) {
    for step in [
        "reminders_external",
        "reminders_crud",
        "reminders_list",
        "reminders_recurring",
        "reminders_permission_loss",
    ] {
        registry.register(step, run);
    }
    registry.register("reminders_store", |world, _, evidence| {
        crate::reminders_store::run(world, evidence)
    });
    registry.register("reminders_store_perf", |world, _, evidence| {
        crate::reminders_store::run(world, evidence)
    });
    registry.register("latency", |world, input, evidence| {
        if evidence.scenario == "REM-STORE-PERF" {
            Ok(())
        } else {
            crate::live_latency::latency(world, input, evidence)
        }
    });
    registry.register("fixture", |_, _, _| Ok(()));
}

#[must_use]
pub fn starts_world(id: &str) -> bool {
    SCENARIOS.contains(&id)
        || crate::reminders_dates::SCENARIOS.contains(&id)
        || crate::reminders_store::SCENARIOS.contains(&id)
}

pub fn start_world(world: &mut World) -> Result<()> {
    world.control("/seed", &json!({"count": 0}))?;
    let gateway = gateway_client(world)?;
    if !await_until(10, || {
        records(&gateway).is_ok_and(|(_, all)| all.len() >= 2)
    }) {
        return Err(Error::Invalid("seed not captured".into()));
    }
    replicas::start(world, 1)
}

pub fn run(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if crate::reminders_dates::SCENARIOS.contains(&id.as_str()) {
        return crate::reminders_dates::run(world, evidence);
    }
    if crate::reminders_store::SCENARIOS.contains(&id.as_str()) {
        return crate::reminders_store::run(world, evidence);
    }
    if !SCENARIOS.contains(&id.as_str()) {
        return Err(Error::Invalid(format!("native scenario {id} not ported")));
    }
    let mut native = Native {
        world: &*world,
        gateway: gateway_client(world)?,
        replicas: replicas::endpoints(),
        mcp: false,
    };
    let mut out = Out {
        evidence,
        prefix: "",
    };
    if !native.protocol(&id, &mut out)? || id != "REM-CRUD" || !out.evidence.passed {
        return Ok(());
    }
    native.mcp = true;
    out.prefix = "mcp_";
    native.protocol(&id, &mut out)?;
    Ok(())
}
