#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Observed, await_until, converged, read_all},
    http::Client,
    readmodels::{PROTOCOLS, call},
    replicas,
};
use lab_demo::{Options, generate};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

const SCOPE: &str = "reminders:read,system:read";
const TIME_ZONE: &str = "America/New_York";

fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}

fn without_cursor(value: &Value) -> Value {
    let mut copy = value.clone();
    if let Some(map) = copy.as_object_mut() {
        map.remove("cursor");
    }
    strip_nulls(&copy)
}

fn title(record: &Value) -> String {
    record["data"]["title"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn non_empty(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn records_checks(client: &Client, evidence: &mut Evidence) {
    let records = read_all(client, false, false);
    evidence.check("demo_reminders_read_records", records.is_ok(), "records");
    let records = records.map(|read| read.records).unwrap_or_default();
    let mut parent_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut tags_seen = BTreeSet::new();
    let mut group_ids = BTreeSet::new();
    let mut group_parents = vec![];
    let (mut ordered, mut pinned, mut emblems) = (0, 0, 0);
    let (
        mut lists,
        mut flags,
        mut parents,
        mut sections,
        mut tagged,
        mut smart,
        mut groups,
        mut assigned,
    ) = (0, 0, 0, 0, 0, 0, 0, 0);
    for record in &records {
        let fields = &record["data"];
        let id = record["id"].as_str().unwrap_or_default().to_owned();
        if record["collection"] == "reminders.list" {
            lists += 1;
            if fields["is_group"] == json!(true) {
                groups += 1;
                group_ids.insert(id.clone());
            }
            if let Some(parent) = non_empty(&fields["parent_id"]) {
                group_parents.push(parent.to_owned());
            }
            if fields["display_order"].is_number() {
                ordered += 1;
            }
            if fields["pinned"] == json!(true) {
                pinned += 1;
            }
            if non_empty(&fields["badge_emblem"]).is_some() {
                emblems += 1;
            }
            if !fields["smart_list_type"].is_null() {
                smart += 1;
            }
            sections += fields["sections"].as_array().map_or(0, Vec::len);
        }
        if record["collection"] != "reminders.reminder" {
            continue;
        }
        if fields["flagged"] == json!(true) {
            flags += 1;
        }
        if let Some(parent) = non_empty(&fields["parent_id"]) {
            parents += 1;
            parent_ids.insert(id.clone(), parent.to_owned());
        }
        if let Some(tags) = fields["tags"].as_array().filter(|tags| !tags.is_empty()) {
            tagged += 1;
            for tag in tags.iter().filter_map(Value::as_str) {
                tags_seen.insert(tag.to_owned());
            }
        }
        let delegate = &fields["assignment"]["assignee"];
        if delegate["is_me"] == json!(true) && non_empty(&delegate["id"]).is_some() {
            assigned += 1;
        }
    }
    evidence.check(
        "demo_reminders_lists_and_group",
        lists == 6 && groups == 1,
        "lists",
    );
    let linked = group_parents.len() == 2 && group_parents.iter().all(|p| group_ids.contains(p));
    evidence.check("demo_reminders_group_reference_resolved", linked, "group");
    evidence.check(
        "demo_reminders_native_order_pin_and_emblems",
        ordered == 6 && pinned == 1 && emblems == 6,
        "order",
    );
    evidence.check("demo_reminders_flags", flags == 2, "flags");
    let deep = parent_ids
        .values()
        .any(|parent| parent_ids.get(parent).is_some_and(|p| !p.is_empty()));
    evidence.check(
        "demo_reminders_recursive_subtasks",
        parents == 2 && deep,
        "subtasks",
    );
    evidence.check("demo_reminders_sections", sections == 4, "sections");
    evidence.check(
        "demo_reminders_tags",
        tagged == 9 && tags_seen.len() == 4,
        "tags",
    );
    evidence.check("demo_reminders_smart_list", smart == 1, "smart");
    evidence.check(
        "demo_reminders_source_verified_assignee",
        assigned == 1,
        "assignee",
    );
}

fn row_titles(result: &Value) -> Vec<Vec<String>> {
    result["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|section| {
            section["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| title(&item["reminder"]))
                .collect()
        })
        .collect()
}

fn rows(expected: &[&[&str]]) -> Vec<Vec<String>> {
    expected
        .iter()
        .map(|row| row.iter().map(|t| (*t).to_owned()).collect())
        .collect()
}

fn items_exact(name: &str, result: &Value) -> bool {
    let titles = row_titles(result);
    let count = result["count"].as_i64();
    let sections = result["sections"].as_array().cloned().unwrap_or_default();
    match name {
        "travel" => {
            if count != Some(3) || sections.len() != 1 {
                return false;
            }
            let mut items = sections[0]["items"].as_array().cloned().unwrap_or_default();
            if items.len() != 1 {
                return false;
            }
            for expected in [
                "Plan a weekend trip",
                "Book a place to stay",
                "Compare two quiet cabins",
            ] {
                if items.len() != 1 || title(&items[0]["reminder"]) != expected {
                    return false;
                }
                items = items[0]["subtasks"].as_array().cloned().unwrap_or_default();
            }
            items.is_empty()
        }
        "groceries" => {
            count == Some(3)
                && sections.len() == 2
                && sections[0]["section"]["kind"] == "store"
                && sections[0]["section"]["title"] == "Produce"
                && sections[1]["section"]["kind"] == "store"
                && sections[1]["section"]["title"] == "Pantry"
                && titles
                    == rows(&[
                        &["Pick up dinner ingredients", "Tomatoes and basil"],
                        &["Pasta and olive oil"],
                    ])
        }
        "assigned" => {
            count == Some(1)
                && titles == rows(&[&["Plan a weekend trip"]])
                && sections[0]["items"][0]["subtasks"]
                    .as_array()
                    .is_none_or(Vec::is_empty)
        }
        "personal_incomplete" => {
            count == Some(2) && titles == rows(&[&["Take a long walk"], &["Water the plants"]])
        }
        "personal_completed" => {
            count == Some(3)
                && titles
                    == rows(&[
                        &["Take a long walk", "Choose the next book"],
                        &["Water the plants"],
                    ])
        }
        _ => false,
    }
}

fn model_checks(clients: &[Client], evidence: &mut Evidence) {
    let mut expected_lists: Option<Value> = None;
    let mut expected_items: BTreeMap<String, Value> = BTreeMap::new();
    for (index, client) in clients.iter().enumerate() {
        for protocol in PROTOCOLS {
            let prefix = format!("demo_readmodels_{index}_{protocol}_");
            let lists = call(
                client,
                protocol,
                "reminders.lists",
                &json!({"time_zone": TIME_ZONE}),
            );
            evidence.check(&format!("{prefix}lists_call"), lists.is_some(), "lists");
            let Some(lists) = lists else { continue };
            let stripped = without_cursor(&lists);
            let expected = expected_lists.get_or_insert_with(|| stripped.clone());
            evidence.check(
                &format!("{prefix}lists_parity"),
                *expected == stripped,
                "parity",
            );
            let entries = lists["lists"].as_array().cloned().unwrap_or_default();
            let by_title: BTreeMap<String, Value> = entries
                .iter()
                .map(|entry| (title(&entry["list"]), entry.clone()))
                .collect();
            let order: Vec<String> = entries.iter().map(|entry| title(&entry["list"])).collect();
            evidence.check(
                &format!("{prefix}sidebar_order"),
                order
                    == [
                        "Travel",
                        "Home",
                        "Personal",
                        "Groceries",
                        "Projects",
                        "Flagged errands",
                    ],
                "order",
            );
            let null = Value::Null;
            let home = by_title.get("Home").unwrap_or(&null);
            let mut groups =
                home["count"].is_null() && home["list"]["data"]["is_group"] == json!(true);
            let orders: BTreeMap<&str, f64> = [
                ("Home", 0.0),
                ("Personal", 1.0),
                ("Groceries", 2.0),
                ("Projects", 3.0),
                ("Travel", 4.0),
                ("Flagged errands", 5.0),
            ]
            .into();
            for (name, list) in &by_title {
                groups &= list["order"]
                    .as_f64()
                    .is_some_and(|o| Some(&o) == orders.get(name.as_str()))
                    && list["pinned"] == json!(name == "Travel");
                if name == "Personal" || name == "Groceries" {
                    groups &= !list["group_id"].is_null() && list["group_id"] == home["list"]["id"];
                } else {
                    groups &= list["group_id"].is_null();
                }
            }
            evidence.check(&format!("{prefix}groups_order_and_pin"), groups, "groups");
            for (name, titles) in [
                ("Personal", ["This week", "Later"]),
                ("Groceries", ["Produce", "Pantry"]),
            ] {
                let sections = by_title
                    .get(name)
                    .and_then(|list| list["sections"].as_array().cloned())
                    .unwrap_or_default();
                let exact = sections.len() == titles.len()
                    && sections.iter().enumerate().all(|(i, section)| {
                        section["title"] == titles[i]
                            && section["kind"] == "store"
                            && section["order"].as_f64().is_some_and(|o| {
                                #[allow(clippy::cast_precision_loss)]
                                let want = i as f64;
                                (o - want).abs() < f64::EPSILON
                            })
                    });
                evidence.check(&format!("{prefix}{name}_sections"), exact, "sections");
            }
            let custom = by_title.get("Flagged errands").unwrap_or(&null);
            evidence.check(
                &format!("{prefix}custom_unsupported"),
                custom["status"] == "unsupported" && custom["count"].is_null(),
                "custom",
            );
            let wanted: BTreeMap<&str, i64> = [
                ("scheduled", 4),
                ("flagged", 2),
                ("all", 9),
                ("completed", 1),
                ("assigned", 1),
            ]
            .into();
            let found: BTreeMap<String, i64> = lists["smart"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tile| {
                    tile["id"]
                        .as_str()
                        .is_some_and(|id| wanted.contains_key(id))
                        && non_empty(&tile["status"]).is_none()
                        && tile["count"].is_number()
                })
                .map(|tile| {
                    (
                        tile["id"].as_str().unwrap_or_default().to_owned(),
                        tile["count"].as_i64().unwrap_or(-1),
                    )
                })
                .collect();
            let smart_ok = found.len() == wanted.len()
                && wanted.iter().all(|(id, n)| found.get(*id) == Some(n));
            evidence.check(&format!("{prefix}smart_counts"), smart_ok, "smart");
            let tags: BTreeMap<String, i64> = lists["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|tag| {
                    (
                        tag["name"].as_str().unwrap_or_default().to_owned(),
                        tag["count"].as_i64().unwrap_or(-1),
                    )
                })
                .collect();
            let want_tags: BTreeMap<String, i64> =
                [("home", 2), ("work", 1), ("travel", 3), ("errands", 2)]
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect();
            evidence.check(
                &format!("{prefix}tags_incomplete_exact"),
                lists["tags"].as_array().map_or(0, Vec::len) == 4 && tags == want_tags,
                "tags",
            );
            let list_id = |name: &str| {
                by_title
                    .get(name)
                    .map_or(Value::Null, |l| l["list"]["id"].clone())
            };
            let queries = [
                ("travel", json!({"list": list_id("Travel")})),
                ("groceries", json!({"list": list_id("Groceries")})),
                ("assigned", json!({"smart": "assigned"})),
                ("personal_incomplete", json!({"list": list_id("Personal")})),
                (
                    "personal_completed",
                    json!({"list": list_id("Personal"), "show_completed": true}),
                ),
            ];
            for (name, mut query) in queries {
                query["time_zone"] = json!(TIME_ZONE);
                let items = call(client, protocol, "reminders.items", &query);
                evidence.check(&format!("{prefix}{name}_call"), items.is_some(), "items");
                let Some(items) = items else { continue };
                let stripped = without_cursor(&items);
                let expected = expected_items
                    .entry(name.to_owned())
                    .or_insert_with(|| stripped.clone());
                evidence.check(
                    &format!("{prefix}{name}_parity"),
                    *expected == stripped,
                    "parity",
                );
                evidence.check(
                    &format!("{prefix}{name}_exact"),
                    items_exact(name, &items),
                    "exact",
                );
            }
        }
    }
}

pub fn reminders_demo(world: &World, evidence: &mut Evidence) -> Result<()> {
    let dataset = generate(Options {
        seed: 17,
        huge: false,
    });
    let observations: Vec<Value> = dataset
        .records
        .iter()
        .filter(|record| {
            record["collection"]
                .as_str()
                .is_some_and(|c| c.starts_with("reminders."))
        })
        .cloned()
        .collect();
    evidence.check(
        "generated_reminders_demo_records",
        observations.len() == 16,
        "records",
    );
    if observations.is_empty() {
        return Ok(());
    }
    let started = replicas::start_with(world, 1, SCOPE, 0);
    evidence.check("demo_reminders_world", started.is_ok(), "world");
    started?;
    let loaded = crate::demo::send(
        world,
        "/dataset",
        &json!({"records": observations, "blobs": {}}),
        120,
    );
    evidence.check("demo_reminders_sim_capture", loaded.is_ok(), "dataset");
    loaded?;
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    evidence.check(
        "demo_reminders_replica_convergence",
        await_until(10, || converged(world, &observed)),
        "convergence",
    );
    records_checks(&gateway, evidence);
    let mut clients = vec![gateway];
    clients.extend(replicas::endpoints().iter().map(replicas::Endpoint::client));
    model_checks(&clients, evidence);
    Ok(())
}
