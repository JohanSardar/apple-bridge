#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    helper_real::truth,
    http::{Client, random_id},
};
use lab_runner::{Evidence, Registry};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::fs;

const SCOPE: &str = "reminders:read,reminders:write,system:read";

pub fn register(registry: &mut Registry) {
    registry.register_real("WEB-REM-WRITE", real);
}

struct Owned {
    id: String,
    list: String,
    tag: String,
}

struct Run<'a> {
    world: &'a World,
    client: Client,
    owned: Vec<Owned>,
    requests: Vec<Value>,
}

impl Run<'_> {
    fn journal(&self) -> Result<()> {
        let owners: Vec<Value> = self
            .owned
            .iter()
            .map(|o| json!({"id": o.id, "list": o.list, "tag": o.tag}))
            .collect();
        fs::write(
            self.world.directory.join("web-rem-write-recovery.json"),
            serde_json::to_vec(&json!({"owners": owners, "requests": self.requests}))?,
        )?;
        Ok(())
    }

    fn find(&self, collection: &str, id: &str) -> Option<Value> {
        let read = self
            .client
            .call(
                "records.get",
                &json!({"collection": collection, "id": id}),
                20,
            )
            .ok()??;
        read["records"].as_array()?.iter().find_map(|record| {
            (record["id"] == id && record["deleted"] != true).then(|| record.clone())
        })
    }

    fn submit(&mut self, op: &str, args: &Value, version: u64) -> Result<Value> {
        let body = json!({
            "command_id": random_id("cmd")?,
            "op": op,
            "args": args,
            "expected_version": version,
            "wait_ms": 15000
        });
        self.requests
            .push(json!({"operation": op, "request": body.clone()}));
        self.journal()?;
        let (status, command) = self
            .client
            .call_raw(op, &body, 40)?
            .ok_or_else(|| Error::Invalid("no response".into()))?;
        if status != 200 || !command["error"].is_null() {
            return Err(Error::Invalid(format!("{op} HTTP {status}")));
        }
        if command["disposition"] != "locally_observed"
            || command["observed_record_ids"]
                .as_array()
                .is_none_or(|ids| ids.len() != 1)
        {
            return Err(Error::Invalid(
                "command_observation_identity_missing".into(),
            ));
        }
        Ok(command)
    }

    fn observe(&self, index: usize) -> Result<Vec<Value>> {
        let owned = &self.owned[index];
        let value = truth(
            self.world,
            "observe",
            &json!({"source": "reminders", "list": owned.list, "list_id": owned.id,
                "tag": owned.tag, "full_fields": true}),
        )?;
        value["records"]
            .as_array()
            .cloned()
            .ok_or_else(|| Error::Invalid("truth_records_missing".into()))
    }

    fn cleanup(&self) -> bool {
        for owned in &self.owned {
            let request = json!({"source": "reminders", "action": "cleanup", "list": owned.list,
                "list_id": owned.id, "tag": owned.tag});
            if truth(self.world, "fixture", &request).is_err() {
                return false;
            }
            let mut request = request;
            request["action"] = json!("cleanup_list");
            if truth(self.world, "fixture", &request).is_err() {
                return false;
            }
            let swept = truth(
                self.world,
                "observe",
                &json!({"source": "reminders", "list": owned.list, "list_id": owned.id,
                    "tag": owned.tag, "all_lists": true}),
            );
            if !swept.is_ok_and(|sweep| sweep["list_exists"] == false) {
                return false;
            }
        }
        true
    }
}

fn exact_priority(record: &Value, expected: i64) -> bool {
    record["priority"].as_i64() == Some(expected)
}

pub fn real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let ready = Client::new(world, SCOPE);
    let tag_id = random_id("run")?;
    let tag = format!("[ab-test {tag_id}]");
    let list_name = format!("Bridge Test {tag}");
    let destination_tag = format!("[ab-test {tag_id}-destination]");
    let destination_name = format!("Bridge Test {destination_tag}");
    let prefix = format!("{tag} {destination_tag}");
    let Ok(client) = ready else {
        evidence.check(
            "private_signed_world_and_owned_tagged_fixture",
            false,
            "Native world or AppleTruth Reminders access unavailable; no permission changes requested",
        );
        return Ok(());
    };
    let helper = world.gateway_home.join("helper");
    let _ = fs::create_dir_all(&helper);
    let _ = fs::write(helper.join("lab-pause-media"), b"paused");
    let shared: &World = world;
    let mut run = Run {
        world: shared,
        client,
        owned: vec![],
        requests: vec![],
    };
    let created = truth(
        world,
        "fixture",
        &json!({"source": "reminders", "action": "create_list", "list": list_name, "tag": tag}),
    );
    let first_id = created
        .as_ref()
        .ok()
        .filter(|value| value["created"] == true)
        .and_then(|value| value["id"].as_str())
        .unwrap_or_default()
        .to_owned();
    evidence.check(
        "private_signed_world_and_owned_tagged_fixture",
        !first_id.is_empty(),
        if first_id.is_empty() {
            "Native world or AppleTruth Reminders access unavailable; no permission changes requested"
        } else {
            ""
        },
    );
    if first_id.is_empty() {
        return Ok(());
    }
    run.owned.push(Owned {
        id: first_id,
        list: list_name.clone(),
        tag: tag.clone(),
    });
    let _ = run.journal();
    let _ = journey(
        &mut run,
        evidence,
        &list_name,
        &destination_name,
        &destination_tag,
        &prefix,
        &tag,
    );
    let cleaned = run.cleanup()
        && [(&list_name, &tag), (&destination_name, &destination_tag)]
            .iter()
            .all(|(list, tag)| {
                truth(
                    world,
                    "observe",
                    &json!({"source": "reminders", "list": list, "tag": tag}),
                )
                .is_ok_and(|sweep| sweep["list_exists"] == false)
            });
    if cleaned {
        let _ = fs::remove_file(world.directory.join("web-rem-write-recovery.json"));
    }
    evidence.check("owned_lists_tagged_items_cleanup_and_sweep", cleaned, "");
    Ok(())
}

fn journey(
    run: &mut Run<'_>,
    evidence: &mut Evidence,
    list_name: &str,
    destination_name: &str,
    destination_tag: &str,
    prefix: &str,
    tag: &str,
) -> Result<()> {
    let mut list_a = Value::Null;
    let located = await_until(20, || {
        run.client
            .call(
                "records.query",
                &json!({"collection": "reminders.list", "limit": 1000}),
                20,
            )
            .ok()
            .flatten()
            .and_then(|value| value["records"].as_array().cloned())
            .and_then(|records| {
                records
                    .into_iter()
                    .find(|record| record["data"]["title"] == list_name)
            })
            .is_some_and(|record| {
                list_a = record;
                true
            })
    });
    evidence.check("unique_owned_list_capture", located, "");
    if !located {
        return Ok(());
    }
    let list_a_id = list_a["id"].as_str().unwrap_or_default().to_owned();
    let created_list = run.submit(
        "reminders.create_list",
        &json!({"title": destination_name}),
        0,
    );
    let native_key = created_list
        .as_ref()
        .ok()
        .and_then(|command| command["evidence"]["native_key"].as_str())
        .unwrap_or_default()
        .to_owned();
    let list_ok = created_list.is_ok() && !native_key.is_empty() && native_key != run.owned[0].id;
    evidence.check(
        "create_list_native_identity_and_observed_record_id",
        list_ok,
        "",
    );
    if !list_ok {
        return Ok(());
    }
    run.owned.push(Owned {
        id: native_key,
        list: destination_name.to_owned(),
        tag: destination_tag.to_owned(),
    });
    if run.journal().is_err() {
        evidence.check("record_created_list_for_recovery", false, "");
        return Ok(());
    }
    let list_b = created_list?["observed_record_ids"][0]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let blank = run.submit(
        "reminders.create",
        &json!({"list_id": list_a_id, "title": format!("{prefix} blank fields"), "notes": "",
            "url": "", "due": "", "due_all_day": false, "priority": 0}),
        0,
    );
    evidence.check("create_blank_optional_fields", blank.is_ok(), "");
    let Ok(blank) = blank else {
        return Ok(());
    };
    let blank_exact = run.observe(0).is_ok_and(|records| {
        records.len() == 1
            && records[0]["notes"].is_null()
            && records[0]["url"].is_null()
            && records[0]["due"].is_null()
            && exact_priority(&records[0], 0)
    });
    evidence.check(
        "eventkit_truth_blank_optional_fields_exact",
        blank_exact,
        "",
    );
    if !blank_exact {
        return Ok(());
    }
    let blank_id = blank["observed_record_ids"][0]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let Some(blank_record) = run.find("reminders.reminder", &blank_id) else {
        evidence.check("blank_fixture_capture", false, "");
        return Ok(());
    };
    let deleted = run.submit(
        "reminders.delete",
        &json!({"id": blank_record["id"]}),
        blank_record["version"].as_u64().unwrap_or(0),
    );
    evidence.check("blank_fixture_deleted", deleted.is_ok(), "");
    if deleted.is_err() {
        return Ok(());
    }
    let title = format!("{prefix} web write");
    let created = run.submit(
        "reminders.create",
        &json!({"list_id": list_a_id, "title": title, "notes": format!("{tag} notes"),
            "url": "https://example.com/reminder", "due": "2028-01-02", "due_all_day": true,
            "priority": 1}),
        0,
    );
    evidence.check("create_observed_identity", created.is_ok(), "");
    let Ok(created) = created else {
        return Ok(());
    };
    let mut item_id = created["observed_record_ids"][0]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let exact = run.observe(0).is_ok_and(|records| {
        records.len() == 1 && {
            let record = &records[0];
            record["title"] == title.as_str()
                && record["notes"] == format!("{tag} notes").as_str()
                && record["url"] == "https://example.com/reminder"
                && exact_priority(record, 1)
                && record["due"]["year"].as_i64() == Some(2028)
                && record["due"]["month"].as_i64() == Some(1)
                && record["due"]["day"].as_i64() == Some(2)
        }
    });
    evidence.check("eventkit_truth_create_fields_exact", exact, "");
    if !exact {
        return Ok(());
    }
    let current = run.find("reminders.reminder", &item_id);
    evidence.check("observed_id_locates_created_record", current.is_some(), "");
    let Some(current) = current else {
        return Ok(());
    };
    let title = format!("{prefix} edited");
    let edited = run.submit(
        "reminders.update",
        &json!({"id": item_id, "title": title, "notes": format!("{tag} edited notes"),
            "url": "", "due": "", "due_all_day": false, "priority": 5}),
        current["version"].as_u64().unwrap_or(0),
    );
    evidence.check("edit_and_clear_date_url", edited.is_ok(), "");
    if edited.is_err() {
        return Ok(());
    }
    let exact = run.observe(0).is_ok_and(|records| {
        records.len() == 1 && {
            let record = &records[0];
            record["title"] == title.as_str()
                && record["notes"] == format!("{tag} edited notes").as_str()
                && record["due"].is_null()
                && record["url"].is_null()
                && exact_priority(record, 5)
        }
    });
    evidence.check("eventkit_truth_edited_fields_and_clears_exact", exact, "");
    if !exact {
        return Ok(());
    }
    for op in ["reminders.complete", "reminders.uncomplete"] {
        let Some(current) = run.find("reminders.reminder", &item_id) else {
            evidence.check(&format!("fresh_version_before_{op}"), false, "");
            return Ok(());
        };
        let done = run.submit(
            op,
            &json!({"id": item_id}),
            current["version"].as_u64().unwrap_or(0),
        );
        evidence.check(&format!("{op}_observed"), done.is_ok(), "");
        if done.is_err() {
            return Ok(());
        }
        let exact = run.observe(0).is_ok_and(|records| {
            records.len() == 1 && records[0]["completed"] == (op == "reminders.complete")
        });
        evidence.check(&format!("{op}_eventkit_truth"), exact, "");
        if !exact {
            return Ok(());
        }
    }
    let Some(current) = run.find("reminders.reminder", &item_id) else {
        evidence.check("fresh_version_before_move", false, "");
        return Ok(());
    };
    let moved = run.submit(
        "reminders.update",
        &json!({"id": item_id, "list_id": list_b, "notes": ""}),
        current["version"].as_u64().unwrap_or(0),
    );
    evidence.check("move_observed_identity", moved.is_ok(), "");
    let Ok(moved) = moved else {
        return Ok(());
    };
    moved["observed_record_ids"][0]
        .as_str()
        .unwrap_or_default()
        .clone_into(&mut item_id);
    let source = run.observe(0);
    let destination = run.observe(1);
    let moved_exact =
        matches!((&source, &destination), (Ok(s), Ok(d)) if s.is_empty() && d.len() == 1);
    evidence.check("eventkit_truth_move_exact", moved_exact, "");
    if !moved_exact {
        return Ok(());
    }
    let destination = destination?;
    let notes = &destination[0]["notes"];
    evidence.check(
        "eventkit_truth_notes_cleared_on_move",
        notes.is_null() || notes == "",
        "",
    );
    let Some(current) = run.find("reminders.reminder", &item_id) else {
        evidence.check("fresh_version_before_delete", false, "");
        return Ok(());
    };
    let deleted = run.submit(
        "reminders.delete",
        &json!({"id": item_id}),
        current["version"].as_u64().unwrap_or(0),
    );
    evidence.check("delete_observed_identity", deleted.is_ok(), "");
    if deleted.is_err() {
        return Ok(());
    }
    evidence.check(
        "eventkit_truth_delete_exact",
        run.observe(1).is_ok_and(|records| records.is_empty()),
        "",
    );
    Ok(())
}
