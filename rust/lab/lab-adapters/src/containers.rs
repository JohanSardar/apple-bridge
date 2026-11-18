use crate::{
    assertions::{await_until, read_all},
    helper_real::truth,
    http::{Client, random_id},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf};

pub const CHECK: &str = "created_containers_removed";
pub const SCENARIO: &str = "LAB-CREATED-CONTAINERS";
const UNREADABLE: &str = "the run record of created containers could not be read";
const SCOPE: &str = "calendar:read,calendar:write,reminders:read,reminders:write,notes:read,notes:write,system:read";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Calendar,
    ReminderList,
    NotesFolder,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Calendar, Self::ReminderList, Self::NotesFolder];
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::ReminderList => "reminder_list",
            Self::NotesFolder => "notes_folder",
        }
    }
    #[must_use]
    pub fn collection(self) -> &'static str {
        match self {
            Self::Calendar => "calendar.calendar",
            Self::ReminderList => "reminders.list",
            Self::NotesFolder => "notes.folder",
        }
    }
    fn create(self) -> &'static str {
        match self {
            Self::Calendar => "calendar.create_calendar",
            Self::ReminderList => "reminders.create_list",
            Self::NotesFolder => "notes.create_folder",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

#[must_use]
pub fn title(tag: &str) -> String {
    format!("Bridge Test {tag}")
}

fn valid_tag(tag: &str) -> bool {
    tag.starts_with("[ab-test ") && tag.ends_with(']') && tag.len() > 11 && !tag.contains('\n')
}

#[derive(Default)]
struct Entries {
    expected: Vec<(Kind, String)>,
    created: Vec<(Kind, String, String)>,
}

impl Entries {
    fn kinds(&self) -> Vec<Kind> {
        let mut kinds: Vec<Kind> = self.expected.iter().map(|(kind, _)| *kind).collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }
    fn tags(&self) -> Vec<&str> {
        let mut tags: Vec<&str> = self.expected.iter().map(|(_, tag)| tag.as_str()).collect();
        tags.sort_unstable();
        tags.dedup();
        tags
    }
}

#[derive(Debug)]
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    #[must_use]
    pub fn of(world: &World) -> Self {
        Self::named(world, "created-containers")
    }
    #[must_use]
    pub fn named(world: &World, name: &str) -> Self {
        Self {
            path: world.directory.join(format!("{name}.json")),
        }
    }
    fn load(&self) -> Result<Entries> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Entries::default());
            }
            Err(error) => return Err(error.into()),
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        let kind = |entry: &Value| {
            Kind::parse(entry["kind"].as_str().unwrap_or_default())
                .ok_or_else(|| Error::Invalid("ledger kind".into()))
        };
        let text = |entry: &Value, key: &str| entry[key].as_str().unwrap_or_default().to_owned();
        let mut entries = Entries::default();
        for entry in value["expected"].as_array().into_iter().flatten() {
            entries.expected.push((kind(entry)?, text(entry, "tag")));
        }
        for entry in value["created"].as_array().into_iter().flatten() {
            entries
                .created
                .push((kind(entry)?, text(entry, "tag"), text(entry, "id")));
        }
        Ok(entries)
    }
    fn store(&self, entries: &Entries) -> Result<()> {
        let value = json!({
            "expected": entries.expected.iter().map(|(kind, tag)| json!({"kind": kind.name(), "tag": tag})).collect::<Vec<_>>(),
            "created": entries.created.iter().map(|(kind, tag, id)| json!({"kind": kind.name(), "tag": tag, "id": id})).collect::<Vec<_>>(),
        });
        let temporary = self.path.with_extension("tmp");
        fs::write(&temporary, serde_json::to_vec(&value)?)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }
    pub fn expect(&self, kind: Kind, tag: &str) -> Result<()> {
        if !valid_tag(tag) {
            return Err(Error::Invalid(
                "container ledger needs an [ab-test …] tag".into(),
            ));
        }
        let mut entries = self.load()?;
        if !entries.expected.iter().any(|(k, t)| *k == kind && t == tag) {
            entries.expected.push((kind, tag.to_owned()));
            self.store(&entries)?;
        }
        Ok(())
    }
    pub fn created(&self, kind: Kind, tag: &str, id: &str) -> Result<()> {
        if id.is_empty() {
            return Err(Error::Invalid("created container has no native id".into()));
        }
        self.expect(kind, tag)?;
        let mut entries = self.load()?;
        if !entries
            .created
            .iter()
            .any(|(k, t, i)| *k == kind && t == tag && i == id)
        {
            entries.created.push((kind, tag.to_owned(), id.to_owned()));
            self.store(&entries)?;
        }
        Ok(())
    }
    pub fn observed(&self, gateway: &Client, kind: Kind, tag: &str, title: &str) -> Result<String> {
        self.expect(kind, tag)?;
        let read = read_all(gateway, false, true)?;
        let matches: Vec<&Value> = read
            .records
            .iter()
            .filter(|r| {
                r["collection"] == kind.collection()
                    && r["deleted"] != true
                    && r["data"]["title"] == title
            })
            .collect();
        let [record] = matches.as_slice() else {
            return Err(Error::Invalid(
                "created container not uniquely observed".into(),
            ));
        };
        let keys: Vec<&str> = read
            .aliases
            .iter()
            .filter(|a| a["record_id"] == record["id"] && a["collection"] == kind.collection())
            .filter_map(|a| a["native_key"].as_str())
            .collect();
        let [key] = keys.as_slice() else {
            return Err(Error::Invalid("created container alias missing".into()));
        };
        self.created(kind, tag, key)?;
        Ok((*key).to_owned())
    }
    pub fn record(
        &self,
        gateway: &Client,
        kind: Kind,
        tag: &str,
        title: &str,
        reply: &Value,
    ) -> Result<String> {
        if let Some(key) = reply["evidence"]["native_key"]
            .as_str()
            .filter(|key| !key.is_empty())
        {
            self.created(kind, tag, key)?;
            return Ok(key.to_owned());
        }
        let mut found = Err(Error::Invalid("created container not observed".into()));
        await_until(10, || {
            found = self.observed(gateway, kind, tag, title);
            found.is_ok()
        });
        found
    }
    fn remove(world: &World, kind: Kind, tag: &str, id: &str) -> Result<()> {
        let request =
            json!({"action": "remove_created", "kind": kind.name(), "tag": tag, "id": id});
        if world.helper_app.is_some() {
            let mut request = request;
            request["source"] = json!("containers");
            truth(world, "fixture", &request).map(drop)
        } else {
            world.control("/containers", &request).map(drop)
        }
    }
    fn remaining(world: &World, entries: &Entries) -> Result<BTreeMap<Kind, u64>> {
        let request = json!({
            "action": "remaining",
            "kinds": entries.kinds().iter().map(|kind| kind.name()).collect::<Vec<_>>(),
            "tags": entries.tags(),
            "created": entries.created.iter().map(|(kind, _, id)| json!({"kind": kind.name(), "id": id})).collect::<Vec<_>>(),
        });
        let counts = if world.helper_app.is_some() {
            let mut request = request;
            request["source"] = json!("containers");
            truth(world, "observe", &request)?
        } else {
            world.control("/containers", &request)?
        };
        entries
            .kinds()
            .into_iter()
            .map(|kind| {
                counts[kind.name()]
                    .as_u64()
                    .map(|count| (kind, count))
                    .ok_or_else(|| Error::Invalid("remaining count missing".into()))
            })
            .collect()
    }
    pub fn settle(&self, world: &World, evidence: &mut Evidence, check: &str) -> Result<()> {
        self.report(world, evidence, check, true);
        Ok(())
    }
    pub fn audit(&self, world: &World, evidence: &mut Evidence, check: &str) -> Result<()> {
        self.report(world, evidence, check, false);
        Ok(())
    }
    fn report(&self, world: &World, evidence: &mut Evidence, check: &str, remove: bool) {
        let Ok(entries) = self.load() else {
            evidence.check(check, false, UNREADABLE);
            return;
        };
        if entries.expected.is_empty() {
            return;
        }
        let failures = if remove {
            entries
                .created
                .iter()
                .filter(|(kind, tag, id)| Self::remove(world, *kind, tag, id).is_err())
                .count()
        } else {
            0
        };
        let Ok(counts) = Self::remaining(world, &entries) else {
            evidence.unverified(
                check,
                "created containers could not be observed after cleanup",
            );
            evidence.passed = false;
            return;
        };
        let left: u64 = counts.values().sum();
        let mut detail: Vec<String> = counts
            .iter()
            .map(|(kind, count)| format!("{}={count}", kind.name()))
            .collect();
        detail.push(format!("removal_failures={failures}"));
        evidence.check(check, failures == 0 && left == 0, &detail.join(" "));
    }
}

pub fn settle(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    Ledger::of(world).settle(world, evidence, CHECK)
}

fn create(gateway: &Client, kind: Kind, title: &str) -> Result<String> {
    let id = random_id("cmd")?;
    let body = json!({"command_id": id, "op": kind.create(), "args": {"title": title},
        "expected_version": 0, "wait_ms": 15000});
    let (status, mut reply) = gateway
        .call_raw(kind.create(), &body, 40)?
        .ok_or_else(|| Error::Invalid("create reply".into()))?;
    if status != 200 || !reply["error"].is_null() {
        return Err(Error::Invalid(format!("{} HTTP {status}", kind.create())));
    }
    await_until(30, || {
        if reply["disposition"] == "locally_observed" {
            return true;
        }
        if let Ok(Some(current)) = gateway.call("commands.status", &json!({"command_id": id}), 20) {
            reply = current;
        }
        reply["disposition"] == "locally_observed"
    });
    reply["evidence"]["native_key"]
        .as_str()
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("created container identity missing".into()))
}

fn probe(evidence: &Evidence) -> Evidence {
    Evidence {
        scenario: evidence.scenario.clone(),
        passed: true,
        assertions: vec![],
    }
}

fn caught(evidence: &mut Evidence, name: &str, probe: &Evidence, expected: &str) {
    let observed = probe
        .assertions
        .iter()
        .find(|a| a.name == CHECK)
        .map_or("no check", |a| a.detail.as_str());
    let failed = !probe.passed
        && probe
            .assertions
            .iter()
            .all(|a| a.name != CHECK || !a.passed);
    evidence.check(name, failed && observed == expected, observed);
}

pub fn scenario(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    let tag = format!("[ab-test {}]", random_id("box")?);
    let leftover = format!("[ab-test {}]", random_id("leftover")?);
    let unrecorded = format!("{} unrecorded", title(&tag));
    let container = |key: &str, collection: &str, title: &str| {
        json!({"kind": "upsert", "collection": collection, "native_key": key,
            "data": {"title": title, "writable": true, "account": "sim", "type": "local", "color": "#00aaff"}})
    };
    world.control(
        "/dataset",
        &json!({"records": [
            container("leftover-calendar", "calendar.calendar", &title(&leftover)),
            container("leftover-list", "reminders.list", &title(&leftover)),
            container("leftover-folder", "notes.folder", &title(&leftover)),
            container("unrecorded-calendar", "calendar.calendar", &unrecorded),
        ]}),
    )?;
    let ledger = Ledger::of(world);
    let skipped = Ledger::named(world, "skipped-cleanup");
    for kind in Kind::ALL {
        ledger.expect(kind, &tag)?;
        skipped.expect(kind, &tag)?;
        let key = create(&gateway, kind, &title(&tag));
        evidence.check(
            &format!("{}_created_and_recorded", kind.name()),
            key.is_ok(),
            "native identity recorded before cleanup",
        );
        let key = key?;
        ledger.created(kind, &tag, &key)?;
        skipped.created(kind, &tag, &key)?;
    }
    let counted = "calendar=2 reminder_list=1 notes_folder=1 removal_failures=0";
    let mut skip = probe(evidence);
    skipped.audit(world, &mut skip, CHECK)?;
    caught(evidence, "skipped_cleanup_fails_check", &skip, counted);
    world.control("/fault", &json!({"name": "container-removal-refused"}))?;
    let mut refused = probe(evidence);
    let outcome = skipped.settle(world, &mut refused, CHECK);
    world.control("/fault", &json!({"name": ""}))?;
    outcome?;
    caught(
        evidence,
        "failed_cleanup_fails_check",
        &refused,
        "calendar=2 reminder_list=1 notes_folder=1 removal_failures=3",
    );
    world.control(
        "/external-change",
        &json!({"key": "unrecorded-calendar", "collection": "calendar.calendar", "delete": true}),
    )?;
    ledger.settle(world, evidence, &format!("normal_path_{CHECK}"))?;
    let foreign = Ledger::named(world, "foreign-container");
    foreign.created(Kind::ReminderList, &tag, "leftover-list")?;
    let mut denied = probe(evidence);
    foreign.settle(world, &mut denied, CHECK)?;
    caught(
        evidence,
        "foreign_container_never_removed",
        &denied,
        "reminder_list=1 removal_failures=1",
    );
    let leftovers = Ledger::named(world, "leftovers");
    for kind in Kind::ALL {
        leftovers.expect(kind, &leftover)?;
    }
    let mut untouched = probe(evidence);
    leftovers.audit(world, &mut untouched, CHECK)?;
    caught(
        evidence,
        "preexisting_containers_untouched",
        &untouched,
        "calendar=1 reminder_list=1 notes_folder=1 removal_failures=0",
    );
    let corrupt = Ledger::named(world, "corrupt-record");
    fs::write(&corrupt.path, b"{\"created\": [")?;
    let mut garbled = probe(evidence);
    corrupt.settle(world, &mut garbled, CHECK)?;
    caught(evidence, "corrupt_record_fails_check", &garbled, UNREADABLE);
    let blocked = Ledger::named(world, "unreadable-record");
    fs::create_dir(&blocked.path)?;
    let mut unreadable = probe(evidence);
    blocked.settle(world, &mut unreadable, CHECK)?;
    caught(
        evidence,
        "unreadable_record_fails_check",
        &unreadable,
        UNREADABLE,
    );
    Ok(())
}
