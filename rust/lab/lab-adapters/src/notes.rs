#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::ref_option,
    clippy::semicolon_if_nothing_returned,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::unnecessary_wraps
)]
use crate::{
    assertions::{Observed, await_until, converged, gateway_aliases, read_all},
    http::{Client, random_id},
    replicas::{self, Endpoint},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const SCOPE: &str = "notes:read,notes:write,reminders:read,system:read";
pub const SCENARIOS: [&str; 9] = [
    "NOTE-CRUD",
    "NOTE-DECODE-FULL",
    "NOTE-DECODE",
    "NOTE-EDIT-PARITY",
    "NOTE-EXTERNAL",
    "NOTE-EXTRAS",
    "NOTE-IMPORT",
    "NOTE-LIST",
    "NOTE-RICH-REJECT",
];

fn add(evidence: &mut Evidence, name: &str, ok: bool) {
    evidence.check(name, ok, "");
}
fn command(operation: &str, args: &Value, version: u64) -> Result<(String, Value)> {
    let id = random_id("cmd")?;
    let body = json!({
        "command_id": id, "op": operation, "args": args,
        "expected_version": version, "wait_ms": 15000
    });
    Ok((id, body))
}
fn send(
    client: &Client,
    protocol: &str,
    operation: &str,
    args: &Value,
    version: u64,
) -> Result<(String, Option<(u16, Value)>)> {
    let (id, body) = command(operation, args, version)?;
    let reply = if protocol == "rest" {
        client.call_raw(operation, &body, 30)?
    } else {
        client.call_mcp_raw(operation, &body, 30)?
    };
    Ok((id, reply))
}
fn is_observed(reply: &Option<(u16, Value)>) -> bool {
    reply.as_ref().is_some_and(|(status, v)| {
        *status == 200 && v["error"].is_null() && v["state"] == "observed"
    })
}
fn code(reply: &Option<(u16, Value)>) -> &str {
    reply
        .as_ref()
        .and_then(|(_, v)| v["error"]["code"].as_str())
        .unwrap_or_default()
}
fn records(client: &Client) -> Vec<Value> {
    read_all(client, false, false)
        .map(|read| read.records)
        .unwrap_or_default()
}
fn find(client: &Client, collection: &str, title: &str) -> Option<Value> {
    records(client).into_iter().find(|r| {
        r["collection"] == collection
            && r["data"]["title"] == title
            && r["deleted"] != true
            && r["data"]["recently_deleted"] != true
    })
}
fn effect_count(world: &World, id: &str) -> Option<u64> {
    Some(
        world.control("/effects", &Value::Null).ok()?[id]
            .as_u64()
            .unwrap_or(0),
    )
}
fn no_effects(world: &World) -> bool {
    world
        .control("/effects", &Value::Null)
        .is_ok_and(|e| e.as_object().is_some_and(serde_json::Map::is_empty))
}
fn change(world: &World, key: &str, collection: &str, data: &Value) -> bool {
    world
        .control(
            "/external-change",
            &json!({"key": key, "collection": collection, "data": data}),
        )
        .is_ok()
}
fn folder_ref(key: &str) -> Value {
    json!({"$ref": {"collection": "notes.folder", "native_key": key}})
}
pub fn run(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "NOTE-IMPORT" => crate::imports::notes(world, input, evidence),
        "NOTE-CRUD" | "NOTE-RICH-REJECT" | "NOTE-EXTERNAL" => family(world, &id, evidence),
        "NOTE-EXTRAS" => extras(world, evidence),
        "NOTE-DECODE" | "NOTE-DECODE-FULL" => decode(evidence),
        "NOTE-LIST" => list(world, evidence),
        "NOTE-EDIT-PARITY" => parity(world, evidence),
        other => Err(Error::Invalid(format!("notes not ported for {other}"))),
    }
}
fn family(world: &World, id: &str, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    let tag = format!("[ab-test {}]", random_id("notes")?);
    let title = format!("{tag} note");
    if !change(
        world,
        "note-folder",
        "notes.folder",
        &json!({"title":"Bridge Test","account":"sim"}),
    ) {
        add(evidence, "folder_fixture", false);
        return Ok(());
    }
    let rich = id == "NOTE-RICH-REJECT";
    let html = rich.then(|| format!("<div>{title}</div><div><b>synthetic rich body</b></div>"));
    let data = json!({
        "title": title, "plain_text": format!("{title}\nsynthetic body\n"),
        "folder_id": folder_ref("note-folder"), "locked": false,
        "editable": if rich { "none" } else { "plain" }, "content_status": "complete",
        "attachments": [], "html": html, "block_status": "complete",
        "round_trip_status": "unproven"
    });
    if !change(world, "external-note", "notes.note", &data) {
        add(evidence, "native_fixture", false);
        return Ok(());
    }
    let mut note = Value::Null;
    let imported = await_until(30, || {
        find(&gateway, "notes.note", &title).is_some_and(|n| {
            note = n;
            true
        })
    });
    add(evidence, "external_note_imported", imported);
    if !imported {
        return Ok(());
    }
    match id {
        "NOTE-EXTERNAL" => external(
            world,
            &tag,
            note["id"].as_str().unwrap_or_default(),
            evidence,
        ),
        "NOTE-RICH-REJECT" => rich_reject(world, &gateway, &title, note, evidence),
        _ => crud(world, &gateway, &tag, &title, note, evidence),
    }
}
fn rich_reject(
    world: &World,
    gateway: &Client,
    title: &str,
    mut note: Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let (_, reply) = send(
        gateway,
        "rest",
        "notes.update",
        &json!({"id": note["id"], "plain_text": format!("{title}\nreplacement")}),
        note["version"].as_u64().unwrap_or(0),
    )?;
    add(
        evidence,
        "rich_rejected_before_dispatch",
        code(&reply) == "unsupported",
    );
    let message = reply
        .as_ref()
        .and_then(|(_, v)| v["error"]["message"].as_str())
        .unwrap_or_default();
    add(
        evidence,
        "rich_rejection_explains_recovery",
        message.contains("Notes") && message.contains("plain-text"),
    );
    add(evidence, "no_dispatch_effect", no_effects(world));
    for protocol in ["rest", "mcp"] {
        let html = note["data"]["html"].as_str().unwrap_or_default().replace(
            "synthetic rich body",
            &format!("synthetic rich body {protocol}"),
        );
        let (_, edited) = send(
            gateway,
            protocol,
            "notes.update",
            &json!({"id": note["id"], "html": html}),
            note["version"].as_u64().unwrap_or(0),
        )?;
        let observed = is_observed(&edited);
        add(
            evidence,
            &format!("{protocol}_rich_edit_observed"),
            observed,
        );
        if !observed {
            return Ok(());
        }
        add(
            evidence,
            &format!("{protocol}_rich_structure_preserved"),
            await_until(15, || {
                find(gateway, "notes.note", title).is_some_and(|current| {
                    if current["data"]["html"] == html.as_str() {
                        note = current;
                        true
                    } else {
                        false
                    }
                })
            }),
        );
    }
    Ok(())
}
struct Poll {
    gateway: Client,
    id: String,
    title: String,
}
impl Poll {
    fn state(client: &Client, id: &str, title: &str) -> (bool, bool) {
        let page = client
            .call(
                "records.query",
                &json!({"collection":"notes.note","id":id,"limit":1}),
                10,
            )
            .ok()
            .flatten();
        let Some(rows) = page.and_then(|p| p["records"].as_array().cloned()) else {
            return (false, false);
        };
        for record in rows {
            let data = &record["data"];
            if record["id"] == id
                && record["deleted"] != true
                && data["recently_deleted"] != true
                && data["title"] == title
            {
                let text = data["plain_text"].as_str().unwrap_or_default();
                let ready = data["body_status"] != "pending"
                    && data["body_status"] != "unavailable"
                    && text.trim_end_matches('\n') == format!("{title}\nbody");
                return (true, ready);
            }
        }
        (false, false)
    }
}
fn external(world: &World, tag: &str, id: &str, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    let endpoints: Vec<Client> = std::iter::once(gateway.clone())
        .chain(replicas::endpoints().iter().take(1).map(Endpoint::client))
        .collect();
    let mut values: Vec<Vec<f64>> = vec![vec![]; endpoints.len()];
    for sample in 0..20 {
        let title = format!("{tag} edit-{sample}");
        let poll = Poll {
            gateway: gateway.clone(),
            id: id.to_owned(),
            title: title.clone(),
        };
        let _ = &poll.gateway;
        let data = json!({
            "title": title, "plain_text": format!("{title}\nbody\n"), "locked": false,
            "editable": "plain", "folder_id": folder_ref("note-folder")
        });
        let effect = Instant::now();
        if !change(world, "external-note", "notes.note", &data) {
            add(evidence, "external_edits_converge", false);
            return Ok(());
        }
        let mut first: Vec<Option<Duration>> = vec![None; endpoints.len()];
        let deadline = Instant::now() + Duration::from_secs(15);
        while first.iter().any(Option::is_none) && Instant::now() < deadline {
            for (index, client) in endpoints.iter().enumerate() {
                if first[index].is_none() && Poll::state(client, &poll.id, &poll.title).1 {
                    first[index] = Some(effect.elapsed());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for (index, seen) in first.into_iter().enumerate() {
            let Some(seen) = seen else {
                add(evidence, "external_edits_converge", false);
                return Ok(());
            };
            values[index].push(seen.as_secs_f64() * 1000.0);
        }
    }
    add(evidence, "external_edits_converge", true);
    for (index, label) in ["gateway", "replica"].iter().enumerate() {
        let mut samples = values.get(index).cloned().unwrap_or_default();
        samples.sort_by(f64::total_cmp);
        let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
        let p95 = samples
            .get(rank.saturating_sub(1))
            .copied()
            .unwrap_or(f64::MAX);
        add(
            evidence,
            &format!("{label}_external_p95_under_1s"),
            p95 < 1000.0,
        );
    }
    Ok(())
}
fn inspect_native(world: &World, title: &str) -> Option<String> {
    let truth = world.control("/inspect", &Value::Null).ok()?;
    truth.as_array()?.iter().find_map(|o| {
        (o["collection"] == "notes.note" && o["kind"] == "upsert" && o["data"]["title"] == title)
            .then(|| {
                o["data"]["plain_text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
    })
}
fn truth_matches(world: &World, record: &Value) -> bool {
    let title = record["data"]["title"].as_str().unwrap_or_default();
    inspect_native(world, title)
        .is_some_and(|text| record["data"]["plain_text"].as_str().unwrap_or_default() == text)
}
fn rows_visible(gateway: &Client, deleted: bool, id: &Value, version: u64) -> bool {
    let clients = std::iter::once(gateway.clone())
        .chain(replicas::endpoints().into_iter().map(|e| e.client()));
    clients.into_iter().all(|client| {
        let Ok(read) = read_all(&client, false, false) else {
            return false;
        };
        let mut present = false;
        for row in &read.records {
            if row["id"] == *id && row["deleted"] != true {
                if !deleted && row["version"].as_u64().unwrap_or(0) < version {
                    return false;
                }
                present = true;
            }
        }
        present != deleted
    })
}
fn crud(
    world: &World,
    gateway: &Client,
    tag: &str,
    title: &str,
    note: Value,
    evidence: &mut Evidence,
) -> Result<()> {
    for protocol in ["rest", "mcp"] {
        let known: std::collections::BTreeSet<String> = records(gateway)
            .iter()
            .filter(|r| r["collection"] == "notes.folder")
            .filter_map(|r| r["id"].as_str().map(str::to_owned))
            .collect();
        let folder_title = format!("Bridge Test {tag} {protocol}");
        let (_, reply) = send(
            gateway,
            protocol,
            "notes.create_folder",
            &json!({"title": folder_title}),
            0,
        )?;
        let observed = is_observed(&reply);
        add(
            evidence,
            &format!("{protocol}_create_folder_observed"),
            observed,
        );
        if !observed {
            return Ok(());
        }
        let mut folder = String::new();
        let visible = await_until(5, || {
            records(gateway)
                .iter()
                .find(|r| {
                    r["collection"] == "notes.folder"
                        && r["deleted"] != true
                        && r["data"]["title"] == folder_title.as_str()
                        && !known.contains(r["id"].as_str().unwrap_or_default())
                })
                .is_some_and(|r| {
                    r["id"].as_str().unwrap_or_default().clone_into(&mut folder);
                    true
                })
        });
        if !visible {
            add(evidence, "created_folder_visible", false);
            return Ok(());
        }
        let new_title = format!("{tag} {protocol}");
        let (_, created_reply) = send(
            gateway,
            protocol,
            "notes.create",
            &json!({"title": new_title, "folder_id": folder, "plain_text": "body"}),
            0,
        )?;
        let observed = is_observed(&created_reply);
        add(evidence, &format!("{protocol}_create_observed"), observed);
        if !observed {
            return Ok(());
        }
        let mut created = Value::Null;
        if !await_until(30, || {
            find(gateway, "notes.note", &new_title).is_some_and(|n| {
                created = n;
                true
            })
        }) {
            add(evidence, "created_visible", false);
            return Ok(());
        }
        add(
            evidence,
            &format!("{protocol}_create_matches_truth"),
            truth_matches(world, &created),
        );
        let native_key = created_reply
            .as_ref()
            .and_then(|(_, v)| v["evidence"]["native_key"].as_str())
            .unwrap_or_default()
            .to_owned();
        let metadata = world
            .control("/inspect", &Value::Null)
            .ok()
            .and_then(|truth| {
                truth.as_array()?.iter().find_map(|o| {
                    (o["native_key"] == native_key.as_str()).then(|| o["data"].clone())
                })
            });
        let Some(mut metadata) = metadata.filter(Value::is_object) else {
            add(evidence, &format!("{protocol}_hydration_fixture"), false);
            return Ok(());
        };
        metadata["native_fingerprint"] = json!("ab-test-content-fingerprint");
        metadata["body_status"] = json!("pending");
        metadata["content_status"] = json!("partial");
        metadata["editable"] = json!("none");
        let previous = created["version"].as_u64().unwrap_or(0);
        if !change(world, &native_key, "notes.note", &metadata)
            || !await_until(5, || {
                find(gateway, "notes.note", &new_title).is_some_and(|current| {
                    if current["version"].as_u64().unwrap_or(0) > previous {
                        created = current;
                        true
                    } else {
                        false
                    }
                })
            })
        {
            add(evidence, &format!("{protocol}_metadata_visible"), false);
            return Ok(());
        }
        let (pending_id, pending) = send(
            gateway,
            protocol,
            "notes.update",
            &json!({"id": created["id"], "mode": "append", "plain_text": "\npending"}),
            created["version"].as_u64().unwrap_or(0),
        )?;
        add(
            evidence,
            &format!("{protocol}_pending_content_rejected"),
            code(&pending) == "unsupported",
        );
        add(
            evidence,
            &format!("{protocol}_pending_content_no_effect"),
            effect_count(world, &pending_id) == Some(0),
        );
        metadata["content_status"] = json!("complete");
        metadata["editable"] = json!("plain");
        if let Some(map) = metadata.as_object_mut() {
            map.remove("body_status");
        }
        let advanced = change(world, &native_key, "notes.note", &metadata)
            && await_until(5, || {
                find(gateway, "notes.note", &new_title).is_some_and(|current| {
                    current["version"].as_u64().unwrap_or(0)
                        > created["version"].as_u64().unwrap_or(0)
                })
            });
        add(
            evidence,
            &format!("{protocol}_hydration_advances_revision"),
            advanced,
        );
        let stale_version = created["version"].as_u64().unwrap_or(0);
        for mode in ["append", "replace"] {
            let replacement = if mode == "replace" {
                format!("{new_title}\nreplaced")
            } else {
                "\nappended".to_owned()
            };
            let args = json!({"id": created["id"], "mode": mode, "plain_text": replacement});
            let (_, reply) = send(
                gateway,
                protocol,
                "notes.update",
                &args,
                created["version"].as_u64().unwrap_or(0),
            )?;
            let observed = is_observed(&reply);
            add(evidence, &format!("{protocol}_{mode}_observed"), observed);
            if !observed {
                return Ok(());
            }
            let floor = created["version"].as_u64().unwrap_or(0);
            let visible = await_until(30, || {
                find(gateway, "notes.note", &new_title).is_some_and(|current| {
                    if current["version"].as_u64().unwrap_or(0) > floor {
                        created = current;
                        true
                    } else {
                        false
                    }
                })
            });
            add(evidence, &format!("{protocol}_{mode}_visible"), visible);
            add(
                evidence,
                &format!("{protocol}_{mode}_matches_truth"),
                truth_matches(world, &created),
            );
            let actual = created["data"]["plain_text"].as_str().unwrap_or_default();
            add(
                evidence,
                &format!("{protocol}_{mode}_content"),
                actual.contains(replacement.trim()),
            );
            if mode == "append" {
                let (stale_id, rejected) =
                    send(gateway, protocol, "notes.update", &args, stale_version)?;
                add(
                    evidence,
                    &format!("{protocol}_content_change_rejects_stale_version"),
                    code(&rejected) == "conflict",
                );
                add(
                    evidence,
                    &format!("{protocol}_stale_version_no_effect"),
                    effect_count(world, &stale_id) == Some(0),
                );
            }
        }
        let version = created["version"].as_u64().unwrap_or(0);
        let replicated = await_until(30, || rows_visible(gateway, false, &created["id"], version));
        add(
            evidence,
            &format!("{protocol}_before_delete_replicated"),
            replicated,
        );
        if !replicated {
            return Ok(());
        }
        let (delete_id, deleted) = send(
            gateway,
            protocol,
            "notes.delete",
            &json!({"id": created["id"]}),
            version,
        )?;
        add(
            evidence,
            &format!("{protocol}_delete_observed"),
            is_observed(&deleted),
        );
        add(
            evidence,
            &format!("{protocol}_delete_replicated"),
            await_until(30, || rows_visible(gateway, true, &created["id"], 0)),
        );
        add(
            evidence,
            &format!("{protocol}_delete_preserves_other_note"),
            find(gateway, "notes.note", title).is_some(),
        );
        add(
            evidence,
            &format!("{protocol}_delete_applied_once"),
            effect_count(world, &delete_id) == Some(1),
        );
    }
    let _ = &note;
    Ok(())
}
fn extras(world: &World, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    for key in ["extras-a", "extras-b"] {
        if !change(
            world,
            key,
            "notes.folder",
            &json!({"title": format!("Bridge Test [ab-test extras] {key}"), "account": "sim"}),
        ) {
            add(evidence, "fixture", false);
            return Ok(());
        }
    }
    let _ = change(
        world,
        "extras-note",
        "notes.note",
        &json!({
            "title": "[ab-test extras] note", "editable": "plain", "locked": false,
            "plain_text": "synthetic", "pinned": true, "tags": ["#bridge-test"],
            "blocks": [{"type":"list_item","list_style":"checklist","checked":true,"checklist_id":"ab-test-extras"}],
            "folder_id": folder_ref("extras-a")
        }),
    );
    let mut note = Value::Null;
    let mut destination = String::new();
    let ready = await_until(10, || {
        for r in records(&gateway) {
            if r["data"]["title"] == "[ab-test extras] note" {
                note = r.clone();
            }
            if r["data"]["title"] == "Bridge Test [ab-test extras] extras-b" {
                r["id"]
                    .as_str()
                    .unwrap_or_default()
                    .clone_into(&mut destination);
            }
        }
        !note.is_null() && !destination.is_empty() && note["data"]["body_status"] != "pending"
    });
    add(evidence, "tagged_fixture_visible", ready);
    if !ready {
        return Ok(());
    }
    let fields = note["data"].clone();
    add(
        evidence,
        "pin_tag_checklist_read",
        fields["pinned"] == true
            && fields["tags"].as_array().is_some_and(|t| t.len() == 1)
            && fields["blocks"].as_array().is_some_and(|b| b.len() == 1)
            && fields["blocks"][0]["checked"] == true,
    );
    let row = |id: &str| {
        records(&gateway)
            .into_iter()
            .find(|r| r["id"] == id)
            .unwrap_or(Value::Null)
    };
    let folder = row(&destination);
    let (_, renamed) = send(
        &gateway,
        "rest",
        "notes.rename_folder",
        &json!({"id": destination, "title": "Bridge Test [ab-test extras] renamed"}),
        folder["version"].as_u64().unwrap_or(0),
    )?;
    add(evidence, "rename_observed", is_observed(&renamed));
    add(
        evidence,
        "rename_readback",
        await_until(5, || {
            row(&destination)["data"]["title"] == "Bridge Test [ab-test extras] renamed"
        }),
    );
    let args = json!({"id": note["id"], "folder_id": destination});
    let (_, moved) = send(
        &gateway,
        "rest",
        "notes.move",
        &args,
        note["version"].as_u64().unwrap_or(0),
    )?;
    add(evidence, "move_observed", is_observed(&moved));
    let note_id = note["id"].as_str().unwrap_or_default().to_owned();
    add(
        evidence,
        "move_readback",
        await_until(5, || {
            row(&note_id)["data"]["folder_id"] == destination.as_str()
        }),
    );
    let (_, stale) = send(
        &gateway,
        "rest",
        "notes.move",
        &args,
        note["version"].as_u64().unwrap_or(0),
    )?;
    add(evidence, "stale_move_rejected", code(&stale) == "conflict");
    let current_folder = row(&destination);
    let (_, rejected) = send(
        &gateway,
        "rest",
        "notes.rename_folder",
        &json!({"id": destination, "title": "ordinary folder"}),
        current_folder["version"].as_u64().unwrap_or(0),
    )?;
    add(
        evidence,
        "untagged_rename_rejected",
        code(&rejected) == "invalid",
    );
    let mcp_title = "Bridge Test [ab-test extras] mcp";
    let (_, mcp_renamed) = send(
        &gateway,
        "mcp",
        "notes.rename_folder",
        &json!({"id": destination, "title": mcp_title}),
        current_folder["version"].as_u64().unwrap_or(0),
    )?;
    add(evidence, "mcp_rename_observed", is_observed(&mcp_renamed));
    add(
        evidence,
        "mcp_rename_readback",
        await_until(5, || row(&destination)["data"]["title"] == mcp_title),
    );
    let original = fields["folder_id"].as_str().unwrap_or_default().to_owned();
    let note = row(&note_id);
    let (_, mcp_moved) = send(
        &gateway,
        "mcp",
        "notes.move",
        &json!({"id": note_id, "folder_id": original}),
        note["version"].as_u64().unwrap_or(0),
    )?;
    add(evidence, "mcp_move_observed", is_observed(&mcp_moved));
    add(
        evidence,
        "mcp_move_readback",
        await_until(5, || {
            row(&note_id)["data"]["folder_id"] == original.as_str()
        }),
    );
    Ok(())
}
fn decode(evidence: &mut Evidence) -> Result<()> {
    crate::native_core::rust_tests(
        evidence,
        "gzip_protobuf_scrubbed_fixtures",
        "ab-native",
        &["--test", "notes_reader"],
    );
    Ok(())
}
fn parity(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let config = world.config().clone();
    for (index, id) in ["NOTE-CRUD", "NOTE-RICH-REJECT"].into_iter().enumerate() {
        if index > 0 {
            world.close();
            replicas::stop_all();
            let mut next = config.clone();
            next.directory = config.directory.with_file_name(format!(
                "{}-parity",
                config
                    .directory
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("world")
            ));
            *world = World::start(next)?;
        }
        replicas::start_with(world, 1, SCOPE, 0)?;
        let mut part = Evidence {
            scenario: id.to_owned(),
            passed: true,
            assertions: vec![],
        };
        if let Err(error) = family(world, id, &mut part) {
            part.check("action", false, &error.to_string());
        }
        let mut passed = part.passed;
        for mut assertion in part.assertions {
            assertion.name = format!("{id}/{}", assertion.name);
            passed &= assertion.passed || assertion.status.as_deref() == Some("unverified");
            evidence.passed &=
                assertion.passed || assertion.status.as_deref() == Some("unverified");
            evidence.assertions.push(assertion);
        }
        if !passed {
            break;
        }
    }
    Ok(())
}
fn list(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    replicas::start_with(world, 1, SCOPE, 0)?;
    add(evidence, "world", true);
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let now = jiff::Timestamp::now();
    let days_ago = now
        .checked_sub(jiff::SignedDuration::from_hours(240))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let rfc = |t: jiff::Timestamp| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut fixtures: Vec<(&str, String, Value)> = vec![
        (
            "notes.folder",
            "notes-root".into(),
            json!({"title":"Bridge Test","account":"notes-account","account_name":"Test Account"}),
        ),
        (
            "notes.folder",
            "notes-child".into(),
            json!({"title":"Nested","account":"notes-account","parent_id":folder_ref("notes-root")}),
        ),
        (
            "notes.folder",
            "notes-deleted".into(),
            json!({"title":"Recently Deleted","account":"notes-account","recently_deleted":true}),
        ),
    ];
    for (i, title) in ["Zulu", "Alpha", "Locked", "Deleted"].iter().enumerate() {
        let folder = if i == 3 {
            "notes-deleted"
        } else {
            "notes-child"
        };
        let mut data = json!({
            "title": title, "plain_text": format!("{title}\n[ab-test] Preview 🧪"),
            "folder_id": folder_ref(folder), "created_at": rfc(days_ago),
            "modified_at": rfc(now), "editable": "plain", "content_status": "complete"
        });
        if i == 0 {
            data["pinned"] = json!(true);
            data["attachments"] =
                json!([{"derivatives":{"thumb":{"digest":"b".repeat(64),"state":"present"}}}]);
        }
        if i == 1 {
            data["plain_text"] = json!("");
            data["preview"] = json!("[ab-test] Preview 🧪");
            data["content_status"] = json!("offloaded");
            data["editable"] = json!("none");
        }
        if i == 2 {
            data["locked"] = json!(true);
            data["html"] = json!("private locked body");
            data["blocks"] = json!(["private locked body"]);
            data["thumbnail"] = json!({"digest": "c".repeat(64)});
        }
        fixtures.push(("notes.note", format!("notes-list-{i}"), data));
    }
    let mut ok = true;
    for (collection, key, data) in &fixtures {
        if !change(world, key, collection, data) {
            ok = false;
            break;
        }
    }
    add(
        evidence,
        "captured",
        ok && world.control("/flush", &json!({})).is_ok(),
    );
    add(
        evidence,
        "replica_converged",
        await_until(10, || converged(world, &observed)),
    );
    let aliases = gateway_aliases(&gateway).unwrap_or_default();
    let alias = |collection: &str, key: &str| {
        aliases
            .get(&format!("notes/{collection}/{key}"))
            .cloned()
            .unwrap_or_default()
    };
    let child = alias("notes.folder", "notes-child");
    let deleted_folder = alias("notes.folder", "notes-deleted");
    let thumb = "b".repeat(64);
    let clients = observed.clients();
    for (index, client) in clients.iter().enumerate() {
        let rows = records(client);
        let notes: Vec<&Value> = rows
            .iter()
            .filter(|r| r["collection"] == "notes.note")
            .collect();
        let valid = notes.len() == 4
            && notes.iter().all(|r| {
                let v = r["data"]["content_version"].as_u64().unwrap_or(0);
                v > 0 && v <= r["version"].as_u64().unwrap_or(0)
            });
        add(evidence, &format!("content_versions_{index}"), valid);
        for protocol in ["rest", "mcp"] {
            let prefix = format!("{protocol}_{index}_");
            let call = |c: &Client, input: &Value| -> Result<Option<Value>> {
                if protocol == "rest" {
                    c.call("notes.list", input, 20)
                } else {
                    c.call_mcp("notes.list", input, 20)
                }
            };
            let out = call(
                client,
                &json!({"folder": child, "sort":"modified","time_zone":"UTC"}),
            )?;
            let sections = out
                .as_ref()
                .and_then(|o| o["sections"].as_array().cloned())
                .unwrap_or_default();
            add(
                evidence,
                &format!("{prefix}pinned_date_sections"),
                out.as_ref().is_some_and(|o| o["count"] == 3)
                    && sections.len() == 2
                    && sections[0]["key"] == "pinned"
                    && sections[1]["key"] == "today",
            );
            let (mut locked, mut preview, mut thumbnail, mut nested) = (false, false, false, false);
            for section in &sections {
                for item in section["notes"].as_array().into_iter().flatten() {
                    let fields = &item["note"]["data"];
                    if fields["title"] == "Locked" {
                        locked = item["read_only"] == true
                            && item["preview"] == ""
                            && item["thumbnail"].is_null()
                            && fields["plain_text"].is_null()
                            && fields["html"].is_null()
                            && fields["blocks"].is_null()
                            && fields["thumbnail"].is_null();
                    }
                    if fields["title"] == "Zulu" {
                        preview = item["preview"] == "[ab-test] Preview 🧪";
                        thumbnail = item["thumbnail"]["digest"] == thumb.as_str();
                    }
                }
            }
            for folder in out
                .as_ref()
                .and_then(|o| o["folders"].as_array())
                .into_iter()
                .flatten()
            {
                if folder["folder"]["id"] == child.as_str() {
                    nested = !folder["parent_id"].is_null()
                        && folder["count"] == 3
                        && folder["account_id"] == "notes-account";
                }
            }
            add(evidence, &format!("{prefix}locked_title_only"), locked);
            add(
                evidence,
                &format!("{prefix}preview_thumbnail"),
                preview && thumbnail,
            );
            let accounts = out
                .as_ref()
                .and_then(|o| o["accounts"].as_array().cloned())
                .unwrap_or_default();
            add(
                evidence,
                &format!("{prefix}account_nested_counts"),
                nested && accounts.len() == 1 && accounts[0]["count"] == 3,
            );
            let out = call(client, &json!({"folder": child, "sort":"title"}))?;
            let sections = out
                .as_ref()
                .and_then(|o| o["sections"].as_array().cloned())
                .unwrap_or_default();
            add(
                evidence,
                &format!("{prefix}title_sort"),
                sections.len() == 2
                    && sections[1]["notes"]
                        .as_array()
                        .is_some_and(|n| n.len() == 2)
                    && sections[1]["notes"][0]["note"]["data"]
                        .to_string()
                        .contains("Alpha"),
            );
            let out = call(client, &json!({"folder": child, "sort":"created"}))?;
            let sections = out
                .as_ref()
                .and_then(|o| o["sections"].as_array().cloned())
                .unwrap_or_default();
            add(
                evidence,
                &format!("{prefix}created_date_groups"),
                sections.len() == 2 && sections[1]["key"] == "previous_30_days",
            );
            let out = call(client, &json!({"folder": deleted_folder}))?;
            let sections = out
                .as_ref()
                .and_then(|o| o["sections"].as_array().cloned())
                .unwrap_or_default();
            let single = out.as_ref().is_some_and(|o| o["count"] == 1) && sections.len() == 1;
            add(
                evidence,
                &format!("{prefix}recently_deleted_read_only"),
                single && sections[0]["notes"][0]["read_only"] == true,
            );
            if single {
                let note = &sections[0]["notes"][0]["note"];
                let (_, reply) = send(
                    client,
                    protocol,
                    "notes.delete",
                    &json!({"id": note["id"]}),
                    note["version"].as_u64().unwrap_or(0),
                )?;
                add(
                    evidence,
                    &format!("{prefix}deleted_folder_write_refused"),
                    code(&reply) == "unsupported" && no_effects(world),
                );
            }
            let out = call(client, &json!({}))?;
            add(
                evidence,
                &format!("{prefix}all_excludes_deleted"),
                out.is_some_and(|o| o["count"] == 3),
            );
            add(
                evidence,
                &format!("{prefix}invalid_sort_refused"),
                call(client, &json!({"sort":"invalid"}))?.is_none(),
            );
            add(
                evidence,
                &format!("{prefix}invalid_zone_refused"),
                call(client, &json!({"time_zone":"Invalid/Zone"}))?.is_none(),
            );
            let restricted = if index == 0 {
                Client::new(world, "system:read")?
            } else {
                let endpoint = replicas::endpoints()
                    .get(index - 1)
                    .cloned()
                    .ok_or_else(|| Error::Invalid("replica".into()))?;
                let token = replicas::cli(
                    &endpoint.home,
                    &["token", "create", "--scope", "system:read"],
                )?;
                Client::at(&endpoint.address, &token)
            };
            let out = call(&restricted, &json!({}))?;
            add(
                evidence,
                &format!("{prefix}notes_scope"),
                out.is_some_and(|o| {
                    o["count"] == 0
                        && o["accounts"].as_array().is_none_or(Vec::is_empty)
                        && o["folders"].as_array().is_none_or(Vec::is_empty)
                }),
            );
        }
    }
    Ok(())
}
