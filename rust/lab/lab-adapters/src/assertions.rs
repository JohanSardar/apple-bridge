use crate::{
    http::Client,
    replicas::{self, Endpoint},
    watch::{Cursor, Watcher, record_key},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs, thread,
    time::{Duration, Instant},
};

pub type Aliases = BTreeMap<String, String>;

pub fn await_until(seconds: u64, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}
#[derive(Debug, Clone)]
pub struct Visibility(BTreeSet<String>);
impl Visibility {
    #[must_use]
    pub fn from_scope(scope: &str) -> Self {
        let mut sources: BTreeSet<String> = scope
            .split(',')
            .filter_map(|grant| {
                let (source, access) = grant.split_once(':')?;
                matches!(access, "read" | "write").then(|| source.to_owned())
            })
            .collect();
        if scope.split(',').any(|grant| grant == "admin") {
            sources.extend(
                [
                    "calendar",
                    "contacts",
                    "messages",
                    "notes",
                    "reminders",
                    "system",
                ]
                .map(str::to_owned),
            );
        }
        Self(sources)
    }
    #[must_use]
    pub fn allows_collection(&self, collection: &str) -> bool {
        self.0
            .contains(collection.split('.').next().unwrap_or_default())
    }
    #[must_use]
    pub fn allows(&self, record: &Value) -> bool {
        self.0
            .contains(record["source"].as_str().unwrap_or_default())
    }
}
#[derive(Debug)]
pub struct Read {
    pub cursor: Cursor,
    pub records: Vec<Value>,
    pub aliases: Vec<Value>,
}
pub fn read_all(client: &Client, deleted: bool, aliases: bool) -> Result<Read> {
    let mut out = Read {
        cursor: Cursor::from_value(&Value::Null),
        records: vec![],
        aliases: vec![],
    };
    let mut after = String::new();
    let mut first = true;
    loop {
        let page = client
            .call(
                "records.query",
                &json!({
                    "collection": "",
                    "limit": 1000,
                    "after": after,
                    "include_deleted": deleted,
                    "include_aliases": aliases
                }),
                20,
            )?
            .ok_or_else(|| Error::Invalid("records.query rejected".into()))?;
        if first {
            out.cursor = Cursor::from_value(&page["cursor"]);
            first = false;
        }
        out.records
            .extend(page["records"].as_array().cloned().unwrap_or_default());
        out.aliases
            .extend(page["aliases"].as_array().cloned().unwrap_or_default());
        match page["next"].as_str() {
            Some(next) if !next.is_empty() => next.clone_into(&mut after),
            _ => return Ok(out),
        }
    }
}
pub fn alias_key(source: &str, collection: &str, native: &str) -> String {
    format!("{source}/{collection}/{native}")
}
pub fn collection_source(collection: &str) -> &str {
    collection.split('.').next().unwrap_or_default()
}
pub fn gateway_aliases(gateway: &Client) -> Result<Aliases> {
    let mut out = Aliases::new();
    for alias in read_all(gateway, true, true)?.aliases {
        let key = alias_key(
            alias["source"].as_str().unwrap_or_default(),
            alias["collection"].as_str().unwrap_or_default(),
            alias["native_key"].as_str().unwrap_or_default(),
        );
        let id = alias["record_id"].as_str().unwrap_or_default().to_owned();
        if out.get(&key).is_some_and(|existing| *existing != id) {
            return Err(Error::Invalid("ambiguous alias".into()));
        }
        out.insert(key, id);
    }
    Ok(out)
}
fn resolve_truth(value: &mut Value, source: &str, aliases: &Aliases) -> Result<()> {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_object) {
                let key = alias_key(
                    source,
                    reference
                        .get("collection")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    reference
                        .get("native_key")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                let id = aliases
                    .get(&key)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| Error::Invalid("truth reference has no bridge alias".into()))?;
                *value = Value::String(id.clone());
                return Ok(());
            }
            for item in map.values_mut() {
                resolve_truth(item, source, aliases)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_truth(item, source, aliases)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn truth_records(
    truth: &Value,
    aliases: &Aliases,
    visibility: &Visibility,
) -> Result<Vec<Value>> {
    let mut out = vec![];
    for native in truth.as_array().into_iter().flatten() {
        let collection = native["collection"].as_str().unwrap_or_default();
        if native["kind"] == "absent" || !visibility.allows_collection(collection) {
            continue;
        }
        let source = collection_source(collection);
        let id = aliases
            .get(&alias_key(
                source,
                collection,
                native["native_key"].as_str().unwrap_or_default(),
            ))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Error::Invalid("truth record has no bridge alias".into()))?
            .clone();
        let mut data = native["data"].clone();
        resolve_truth(&mut data, source, aliases)?;
        out.push(json!({
            "id": id, "collection": collection, "source": source,
            "deleted": false, "data": data
        }));
    }
    Ok(out)
}
pub fn record_multiset(records: &[Value], full: bool) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for record in records {
        let collection = record["collection"].as_str().unwrap_or_default();
        if collection == "system.command" || (!full && collection == "system.source") {
            continue;
        }
        let mut data = record["data"].clone();
        if !full
            && collection == "notes.note"
            && let Some(fields) = data.as_object_mut()
        {
            fields.remove("content_version");
        }
        let key = if full {
            let mut whole = record.clone();
            whole["data"] = data;
            whole.to_string()
        } else {
            json!([
                record["id"],
                record["collection"],
                record["source"],
                record["deleted"].as_bool().unwrap_or(false),
                data
            ])
            .to_string()
        };
        *out.entry(key).or_insert(0) += 1;
    }
    out
}
#[derive(Debug, Clone)]
pub struct Observed {
    pub gateway: Client,
    pub gateway_scope: String,
    pub replicas: Vec<Endpoint>,
}
impl Observed {
    #[must_use]
    pub fn new(gateway: Client, gateway_scope: &str) -> Self {
        Self {
            gateway,
            gateway_scope: gateway_scope.to_owned(),
            replicas: replicas::endpoints(),
        }
    }
    #[must_use]
    pub fn clients(&self) -> Vec<Client> {
        std::iter::once(self.gateway.clone())
            .chain(self.replicas.iter().map(Endpoint::client))
            .collect()
    }
}
pub fn truth_matches(world: &World, observed: &Observed) -> bool {
    let Ok(truth) = world.control("/inspect", &Value::Null) else {
        return false;
    };
    let (Ok(gateway), Ok(aliases)) = (
        read_all(&observed.gateway, false, false),
        gateway_aliases(&observed.gateway),
    ) else {
        return false;
    };
    let visibility = Visibility::from_scope(&observed.gateway_scope);
    let Ok(expected) = truth_records(&truth, &aliases, &visibility) else {
        return false;
    };
    record_multiset(&gateway.records, false) == record_multiset(&expected, false)
}
fn replica_scope(endpoint: &Endpoint) -> Visibility {
    Visibility::from_scope(&endpoint.scope)
}
pub fn replica_integrity(observed: &Observed) -> bool {
    let Ok(gateway) = read_all(&observed.gateway, true, false) else {
        return false;
    };
    observed.replicas.iter().all(|endpoint| {
        let Ok(replica) = read_all(&endpoint.client(), true, false) else {
            return false;
        };
        let visibility = replica_scope(endpoint);
        let visible: Vec<Value> = gateway
            .records
            .iter()
            .filter(|record| visibility.allows(record))
            .cloned()
            .collect();
        replica.cursor.epoch == gateway.cursor.epoch
            && replica.cursor.sequence == gateway.cursor.sequence
            && record_multiset(&replica.records, true) == record_multiset(&visible, true)
    })
}
pub fn converged(world: &World, observed: &Observed) -> bool {
    if !truth_matches(world, observed) {
        return false;
    }
    let Ok(gateway) = Client::cursor(&observed.gateway) else {
        return false;
    };
    observed.replicas.iter().all(|endpoint| {
        Client::cursor(&endpoint.client()).is_ok_and(|cursor| {
            cursor.epoch == gateway.epoch && cursor.sequence == gateway.sequence
        })
    }) && replica_integrity(observed)
}
impl Client {
    pub fn cursor(&self) -> Result<Cursor> {
        let page = self
            .call("records.query", &json!({"collection":"","limit":1}), 20)?
            .ok_or_else(|| Error::Invalid("cursor query rejected".into()))?;
        Ok(Cursor::from_value(&page["cursor"]))
    }
}
pub fn record_integrity(
    evidence: &mut Evidence,
    observed: &Observed,
    apply: Option<(bool, String)>,
) {
    if observed.replicas.is_empty() {
        evidence.unverified(
            "replica_records_versions_and_cursor_consistent",
            "no replica checked",
        );
    } else {
        let barrier = observed.gateway.cursor();
        let ok = barrier.is_ok_and(|barrier| {
            await_until(5, || {
                replica_integrity(observed)
                    && observed.gateway.cursor().is_ok_and(|now| {
                        now.epoch == barrier.epoch && now.sequence >= barrier.sequence
                    })
            })
        });
        evidence.check(
            "replica_records_versions_and_cursor_consistent",
            ok,
            "public records query, including versions and tombstones, matched at one full cursor at or beyond the starting watermark",
        );
        mark_status(evidence, ok);
    }
    match apply {
        None => evidence.unverified(
            "replica_application_exactly_once",
            "kill-during-apply check did not run",
        ),
        Some((passed, detail)) => {
            evidence.check("replica_application_exactly_once", passed, &detail);
            mark_status(evidence, passed);
        }
    }
}
fn mark_status(evidence: &mut Evidence, passed: bool) {
    if let Some(last) = evidence.assertions.last_mut() {
        last.status = Some(if passed { "pass" } else { "fail" }.into());
    }
}
pub fn kill_during_apply(world: &World, observed: &Observed, iterations: usize) -> (bool, String) {
    let Some(endpoint) = observed.replicas.first() else {
        return (false, "no replica to crash".into());
    };
    for i in 0..iterations {
        if world.control("/flush", &json!({})).is_err()
            || !await_until(10, || converged(world, observed))
        {
            return (false, format!("pre-crash convergence failed at {i}"));
        }
        let reached = endpoint.home.join("lab-apply-reached");
        let _ = fs::remove_file(&reached);
        if fs::write(endpoint.home.join("lab-apply-armed"), b"").is_err() {
            return (false, "could not arm the apply boundary".into());
        }
        for change in 0..50 {
            if world
                .control(
                    "/external-change",
                    &json!({"key": format!("crash-{i}-{change}")}),
                )
                .is_err()
            {
                return (false, "external change failed".into());
            }
        }
        let boundary = await_until(15, || {
            fs::read_to_string(&reached)
                .is_ok_and(|text| text.trim() == "write_started_before_commit")
        });
        if !boundary {
            return (false, format!("apply boundary not reached at {i}"));
        }
        if replicas::crash(0).is_err() || replicas::restart(0).is_err() {
            return (false, format!("replica restart failed at {i}"));
        }
        if !await_until(15, || {
            replicas::status(endpoint).is_ok_and(|v| v["replica"]["state"] == "connected")
        }) {
            return (false, format!("replica did not reconnect at {i}"));
        }
    }
    if !await_until(30, || converged(world, observed)) {
        return (false, "no convergence after crashes".into());
    }
    let mut homes = vec![];
    for endpoint in &observed.replicas {
        homes.push(endpoint.home.clone());
    }
    let doctor_ok = homes.iter().all(|home| {
        replicas::cli(home, &["doctor"])
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|report| report["ok"] == true && report["exactly_once"] == true)
    });
    (
        doctor_ok,
        format!(
            "{iterations} replica crashes at the apply boundary, doctor exactly_once on every replica"
        ),
    )
}
#[derive(Debug)]
pub struct Observers {
    pub watchers: Vec<Watcher>,
    pub scoped: Vec<Watcher>,
}
pub fn sse_observers(world: &World, observed: &Observed) -> Result<Observers> {
    let mut watchers = vec![];
    for client in observed.clients() {
        watchers.push(Watcher::start(&client, false)?);
    }
    let mut scoped = vec![];
    for (client, token) in scoped_clients(world, observed, "system:read")? {
        let _ = &client;
        scoped.push(Watcher::start(&Client::at(client.address(), &token), true)?);
    }
    Ok(Observers { watchers, scoped })
}
pub fn scoped_clients(
    world: &World,
    observed: &Observed,
    scope: &str,
) -> Result<Vec<(Client, String)>> {
    let mut out = vec![(
        observed.gateway.clone(),
        world
            .gateway_cli(&["token", "create", "--scope", scope])?
            .trim()
            .to_owned(),
    )];
    for endpoint in &observed.replicas {
        out.push((
            endpoint.client(),
            replicas::cli(&endpoint.home, &["token", "create", "--scope", scope])?,
        ));
    }
    Ok(out)
}
pub fn false_deletion_free(world: &World, observed: &Observed, watcher: &Watcher) -> bool {
    let Ok(truth) = world.control("/inspect", &Value::Null) else {
        return false;
    };
    let Ok(aliases) = gateway_aliases(&observed.gateway) else {
        return false;
    };
    let mut absent = BTreeSet::new();
    for native in truth.as_array().into_iter().flatten() {
        if native["kind"] == "absent" {
            let collection = native["collection"].as_str().unwrap_or_default();
            let id = aliases
                .get(&alias_key(
                    collection_source(collection),
                    collection,
                    native["native_key"].as_str().unwrap_or_default(),
                ))
                .cloned()
                .unwrap_or_default();
            absent.insert(format!("{collection}/{id}"));
        }
    }
    watcher
        .deletes()
        .iter()
        .all(|record| absent.contains(&record_key(record)))
}
pub fn stream_assertions(
    world: &World,
    observed: &Observed,
    observers: &Observers,
    evidence: &mut Evidence,
) {
    for (index, watcher) in observers.watchers.iter().enumerate() {
        evidence.check(
            &format!("sse_contiguous_watermarks_{index}"),
            watcher.valid(),
            "every batch continued the previous watermark with ordered sequences",
        );
        evidence.check(
            &format!("no_false_deletion_{index}"),
            false_deletion_free(world, observed, watcher),
            "every observed delete is absent in SimApple truth",
        );
    }
    for (index, watcher) in observers.scoped.iter().enumerate() {
        evidence.check(
            &format!("scope_sse_watermarks_{index}"),
            watcher.valid() && watcher.advanced(),
            "scoped stream carried only system records and advanced",
        );
    }
}
pub fn diagnose(world: &World, observed: &Observed) -> String {
    let truth = world
        .control("/inspect", &Value::Null)
        .unwrap_or(Value::Null);
    let gateway = read_all(&observed.gateway, false, false).map(|r| r.records);
    let aliases = gateway_aliases(&observed.gateway);
    let (Ok(gateway), Ok(aliases)) = (gateway, aliases) else {
        return "read failed".into();
    };
    let visibility = Visibility::from_scope(&observed.gateway_scope);
    let expected = match truth_records(&truth, &aliases, &visibility) {
        Ok(expected) => expected,
        Err(error) => return format!("truth mapping: {error}"),
    };
    if let Ok(full) = read_all(&observed.gateway, true, false) {
        for endpoint in &observed.replicas {
            if let Ok(replica) = read_all(&endpoint.client(), true, false) {
                let visibility = replica_scope(endpoint);
                let visible: Vec<Value> = full
                    .records
                    .iter()
                    .filter(|record| visibility.allows(record))
                    .cloned()
                    .collect();
                let a = record_multiset(&replica.records, true);
                let b = record_multiset(&visible, true);
                let only_replica: Vec<_> =
                    a.keys().filter(|k| !b.contains_key(*k)).take(2).collect();
                let only_gateway: Vec<_> =
                    b.keys().filter(|k| !a.contains_key(*k)).take(2).collect();
                eprintln!(
                    "replica={} visible={} only_replica={:?} only_gateway={:?} scope={}",
                    replica.records.len(),
                    visible.len(),
                    only_replica,
                    only_gateway,
                    endpoint.scope
                );
            }
        }
    }
    let by_id: BTreeMap<String, &Value> = gateway
        .iter()
        .map(|r| (r["id"].as_str().unwrap_or_default().to_owned(), r))
        .collect();
    let mut out = format!(
        "truth={} gateway={} truth_matches={} replica_integrity={}",
        expected.len(),
        gateway.len(),
        truth_matches(world, observed),
        replica_integrity(observed)
    );
    if let Ok(g) = Client::cursor(&observed.gateway) {
        for endpoint in &observed.replicas {
            let _ = write!(
                out,
                " gw_cursor={:?} replica_cursor={:?}",
                g,
                Client::cursor(&endpoint.client()).ok()
            );
        }
    }
    let mut shown = 0;
    for record in &expected {
        let id = record["id"].as_str().unwrap_or_default();
        match by_id.get(id) {
            None => {
                if shown < 3 {
                    let _ = write!(out, "\nmissing {}", record["collection"]);
                    shown += 1;
                }
            }
            Some(actual) if actual["data"] != record["data"] => {
                if shown < 3 {
                    let _ = write!(
                        out,
                        "\ndiff {} truth={} gateway={}",
                        record["collection"], record["data"], actual["data"]
                    );
                    shown += 1;
                }
            }
            Some(_) => {}
        }
    }
    out
}
