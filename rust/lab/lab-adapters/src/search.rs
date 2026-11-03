#![allow(clippy::too_many_lines)]
use crate::{
    assertions::gateway_aliases,
    http::Client,
    readmodels::{PROTOCOLS, call, observation, offset, put, reference},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    thread,
    time::{Duration, Instant},
};

const BASE: &str = "2026-10-20T12:00:00Z";

fn count(value: Option<&Value>) -> Option<usize> {
    value.map(|v| v["items"].as_array().map_or(0, Vec::len))
}
fn fixture(world: &World) -> Result<Vec<Value>> {
    let mut records = vec![
        observation(
            "messages.handle",
            "search-a",
            &json!({"address": "search-a@example.com", "service": "iMessage"}),
        ),
        observation(
            "messages.handle",
            "search-b",
            &json!({"address": "search-b@example.com", "service": "iMessage"}),
        ),
        observation(
            "messages.chat",
            "search-chat",
            &json!({"display_name": "[ab-test] Orchard", "participants": [reference("messages.handle", "search-a"), reference("messages.handle", "search-b")], "style": "group", "service": "iMessage"}),
        ),
    ];
    for index in 0..55_i64 {
        let key = format!("search-message-{index:02}");
        let sender = if index % 2 == 1 {
            "search-b"
        } else {
            "search-a"
        };
        let mut data = json!({"chat_id": reference("messages.chat", "search-chat"), "sender_handle_id": reference("messages.handle", sender), "sent_at": offset(BASE, index * 60)?, "kind": "message", "text": format!("[ab-test] Orchard stone {index}"), "is_from_me": false, "parts": []});
        if index % 3 == 0 {
            data["read_at"] = json!(offset(BASE, 3600)?);
        }
        if index % 5 == 2 {
            data["balloon"] = json!({"kind": "link", "link": {"url": "https://example.com/orchard", "title": "Cliff lantern"}});
        }
        if index % 5 == 3 {
            data["balloon"] = json!({"kind": "find_my", "app": {"caption": "Maple location"}});
        }
        records.push(observation("messages.message", &key, &data));
        if index % 5 < 2 {
            let mime = if index % 5 == 1 {
                "application/pdf"
            } else {
                "image/png"
            };
            records.push(observation(
                "messages.attachment",
                &format!("{key}-attachment"),
                &json!({"message_id": reference("messages.message", &key), "mime_type": mime, "filename": "Canopy.pdf"}),
            ));
        }
        if index % 5 == 4 {
            records.push(observation(
                "messages.attachment",
                &format!("{key}-hidden"),
                &json!({"message_id": reference("messages.message", &key), "mime_type": "image/png", "hidden": true, "filename": "Hidden.pdf"}),
            ));
        }
    }
    for state in [
        "deleted",
        "archived",
        "spam",
        "spam-chat",
        "deleted-message",
        "retracted-message",
    ] {
        let key = format!("search-excluded-{state}");
        let mut chat = json!({"display_name": "[ab-test] Orchard excluded", "participants": [reference("messages.handle", "search-a")], "style": "direct", "service": "iMessage"});
        let mut message = json!({"chat_id": reference("messages.chat", &key), "sent_at": BASE, "kind": "message", "text": "[ab-test] Orchard excluded", "is_from_me": false, "parts": []});
        match state {
            "deleted" => chat["deleted_at"] = json!(BASE),
            "archived" => chat["is_archived"] = json!(true),
            "spam-chat" => chat["filter"] = json!("spam"),
            "deleted-message" => message["deleted_at"] = json!(BASE),
            "retracted-message" => message["retracted_at"] = json!(BASE),
            "spam" => message["is_spam"] = json!(true),
            _ => {}
        }
        records.push(observation("messages.chat", &key, &chat));
        records.push(observation(
            "messages.message",
            &format!("{key}-message"),
            &message,
        ));
    }
    let _ = world;
    Ok(records)
}
fn migration(world: &mut World, evidence_ok: &mut bool) -> Result<()> {
    let gateway = Client::new(world, "messages:read,contacts:read,system:read")?;
    let search = |client: &Client, text: &str| {
        call(
            client,
            "rest",
            "messages.search",
            &json!({"text": text, "limit": 200}),
        )
    };
    let ids = |value: &Value| -> Vec<String> {
        value["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    let before = search(&gateway, "orch").ok_or_else(|| Error::Invalid("search before".into()))?;
    world.stop_gateway();
    let path = world.gateway_home.join("state.db");
    let database = Connection::open(&path).map_err(|e| Error::Invalid(e.to_string()))?;
    database
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let fields = [
        "text",
        "title",
        "notes",
        "plain_text",
        "given_name",
        "middle_name",
        "family_name",
        "nickname",
        "organization",
        "job_title",
    ];
    let expressions: Vec<String> = fields
        .iter()
        .map(|f| format!("coalesce(json_extract(r.payload,'$.data.{f}'),'')"))
        .collect();
    database
        .execute_batch(&format!(
            "DELETE FROM records_fts; INSERT INTO records_fts(rowid,collection,id,source,text) SELECT m.fts_rowid,r.collection,r.id,r.source,{} FROM records r JOIN records_fts_map m ON m.collection=r.collection AND m.id=r.id WHERE r.deleted=0; UPDATE meta SET value='3' WHERE key='fts_version'",
            expressions.join("||' '||")
        ))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let old: i64 = database
        .query_row(
            "SELECT count(*) FROM records_fts WHERE records_fts MATCH ?",
            ["Canop*"],
            |row| row.get(0),
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
    drop(database);
    if old != 0 {
        return Err(Error::Invalid(
            "legacy projection contains filenames".into(),
        ));
    }
    world.restart_gateway()?;
    let gateway = Client::new(world, "messages:read,contacts:read,system:read")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut filenames = None;
    while Instant::now() < deadline {
        filenames = search(&gateway, "Canop");
        if count(filenames.as_ref()) == Some(22) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if count(filenames.as_ref()) != Some(22) {
        return Err(Error::Invalid("filename index did not rebuild".into()));
    }
    let after = search(&gateway, "orch").ok_or_else(|| Error::Invalid("search after".into()))?;
    if ids(&before) != ids(&after) {
        return Err(Error::Invalid("message ids changed".into()));
    }
    let database = Connection::open(&path).map_err(|e| Error::Invalid(e.to_string()))?;
    let version: String = database
        .query_row(
            "SELECT value FROM meta WHERE key='fts_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
    *evidence_ok = version == "5";
    Ok(())
}
pub fn search(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let contact = world.control(
        "/external-change",
        &json!({"key": "search-contact", "collection": "contacts.contact", "data": {"given_name": "Orchard", "family_name": "Keeper", "emails": [{"value": "search-a@example.com"}]}}),
    );
    let contact = contact.and_then(|_| world.control("/flush", &json!({})));
    evidence.check("search_contact_fixture", contact.is_ok(), "contact");
    if contact.is_err() {
        return Ok(());
    }
    let observations = fixture(world)?;
    let mut stored = Ok(());
    for chunk in observations.chunks(64) {
        stored = put(world, chunk);
        if stored.is_err() {
            break;
        }
    }
    evidence.check("synthetic_search_fixture", stored.is_ok(), "fixture");
    if stored.is_err() {
        return Ok(());
    }
    let gateway = Client::new(world, "messages:read,contacts:read,system:read")?;
    let aliases = gateway_aliases(&gateway);
    evidence.check("search_aliases", aliases.is_ok(), "aliases");
    let aliases = aliases.unwrap_or_default();
    let handle = aliases
        .get("messages/messages.handle/search-a")
        .cloned()
        .unwrap_or_default();
    let contact_id = aliases
        .get("contacts/contacts.contact/search-contact")
        .cloned()
        .unwrap_or_default();
    for protocol in PROTOCOLS {
        let read = |input: &Value| call(&gateway, protocol, "messages.search", input);
        let mut ids = std::collections::BTreeSet::new();
        let mut after = String::new();
        let mut pages = 0;
        let mut ordered = true;
        let mut previous: Option<jiff::Timestamp> = None;
        loop {
            let mut input = json!({"text": "orch", "limit": 20});
            if !after.is_empty() {
                input["after"] = json!(after);
            }
            let Some(page) = read(&input) else {
                ordered = false;
                break;
            };
            pages += 1;
            for item in page["items"].as_array().into_iter().flatten() {
                let stamp = item["data"]["sent_at"]
                    .as_str()
                    .and_then(|t| t.parse::<jiff::Timestamp>().ok());
                let id = item["id"].as_str().unwrap_or_default().to_owned();
                if ids.contains(&id) || previous.zip(stamp).is_some_and(|(p, s)| s > p) {
                    ordered = false;
                }
                ids.insert(id);
                previous = stamp;
            }
            match page["next"].as_str() {
                Some(next) if !next.is_empty() => next.clone_into(&mut after),
                _ => break,
            }
            if pages > 4 {
                ordered = false;
                break;
            }
        }
        evidence.check(
            &format!("{protocol}_prefix_pagination_all_55_unique_ordered"),
            ordered && ids.len() == 55 && pages == 3,
            "pagination",
        );
        for kind in ["photos", "documents", "links", "locations"] {
            let page = read(&json!({"text": "orch", "kind": kind, "limit": 200}));
            let valid = page.as_ref().is_some_and(|p| {
                p["items"].as_array().is_some_and(|items| {
                    items.len() == 11
                        && items.iter().all(|item| {
                            item["attachments"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .all(|a| a["data"]["hidden"] != true)
                        })
                })
            });
            evidence.check(
                &format!("{protocol}_kind_{kind}_before_limit_hidden_excluded"),
                valid,
                kind,
            );
        }
        let from =
            read(&json!({"text": "orch", "from_handle_id": handle, "unread": true, "limit": 200}));
        let valid = from.as_ref().is_some_and(|p| {
            p["items"].as_array().is_some_and(|items| {
                items.len() == 18
                    && items.iter().all(|item| {
                        item["data"]["sender_handle_id"] == json!(handle)
                            && item["data"]["read_at"].is_null()
                    })
            })
        });
        evidence.check(
            &format!("{protocol}_from_and_unread_intersection"),
            valid,
            "from",
        );
        let linked = read(&json!({"from_contact_id": contact_id, "limit": 200}));
        evidence.check(
            &format!("{protocol}_contact_linked_sender"),
            count(linked.as_ref()) == Some(28),
            "linked",
        );
        let filename = read(&json!({"text": "Canop", "limit": 200}));
        evidence.check(
            &format!("{protocol}_attachment_filename_prefix"),
            count(filename.as_ref()) == Some(22),
            "filename",
        );
        for term in ["Cliff", "Maple"] {
            let page = read(&json!({"text": term, "limit": 200}));
            evidence.check(
                &format!("{protocol}_balloon_content_{term}"),
                count(page.as_ref()) == Some(11),
                term,
            );
        }
        let page = read(&json!({"text": "orch", "limit": 1}));
        let next = page
            .as_ref()
            .and_then(|p| p["next"].as_str())
            .unwrap_or_default()
            .to_owned();
        let changed = read(&json!({"text": "stone", "after": next}));
        evidence.check(
            &format!("{protocol}_cursor_bound_to_query"),
            !next.is_empty() && changed.is_none(),
            "cursor",
        );
        let invalid = [
            json!({}),
            json!({"text": "x".repeat(513)}),
            json!({"text": "orch", "kind": "invalid"}),
            json!({"text": "orch", "limit": 201}),
            json!({"text": "orch", "from_handle_id": handle, "from_contact_id": contact_id}),
        ];
        for query in &invalid {
            evidence.check(
                &format!("{protocol}_invalid_query_rejected"),
                read(query).is_none(),
                "invalid",
            );
        }
        let literal = read(&json!({"text": "orch OR definitelyabsent"}));
        evidence.check(
            &format!("{protocol}_fts_syntax_is_literal"),
            count(literal.as_ref()) == Some(0),
            "literal",
        );
    }
    let messages_only = Client::new(world, "messages:read")?;
    let rejected = call(
        &messages_only,
        "rest",
        "messages.search",
        &json!({"text": "orch", "from_contact_id": contact_id}),
    );
    evidence.check(
        "contacts_hidden_sender_lookup_rejected",
        rejected.is_none(),
        "rejected",
    );
    let contacts_only = Client::new(world, "contacts:read")?;
    let hidden = call(
        &contacts_only,
        "rest",
        "messages.search",
        &json!({"text": "orch"}),
    );
    evidence.check(
        "messages_visibility",
        count(hidden.as_ref()) == Some(0),
        "visibility",
    );
    let delta = |body: &str, deleted: &Value| {
        Ok::<Value, Error>(observation(
            "messages.message",
            "search-message-54",
            &json!({"chat_id": reference("messages.chat", "search-chat"), "sender_handle_id": reference("messages.handle", "search-a"), "sent_at": offset(BASE, 54 * 60)?, "kind": "message", "text": body, "is_from_me": false, "parts": [], "deleted_at": deleted}),
        ))
    };
    evidence.check(
        "search_text_delta_capture",
        put(world, &[delta("[ab-test] Willow", &Value::Null)?]).is_ok(),
        "delta",
    );
    let mut upgraded = false;
    let migrated = migration(world, &mut upgraded);
    evidence.check(
        "fts_v3_upgrade_preserves_records_and_rebuilds_filename_index",
        migrated.is_ok() && upgraded,
        &migrated.err().map(|e| e.to_string()).unwrap_or_default(),
    );
    if !upgraded {
        return Ok(());
    }
    let client = Client::new(world, "messages:read,contacts:read,system:read")?;
    let find = |text: &str| {
        call(
            &client,
            "rest",
            "messages.search",
            &json!({"text": text, "limit": 200}),
        )
    };
    evidence.check(
        "search_text_delta_removes_old_match",
        count(find("orch").as_ref()) == Some(54),
        "removed",
    );
    evidence.check(
        "search_text_delta_adds_new_match",
        count(find("will").as_ref()) == Some(1),
        "added",
    );
    evidence.check(
        "search_delete_capture",
        put(world, &[delta("[ab-test] Willow", &json!(BASE))?]).is_ok(),
        "delete",
    );
    evidence.check(
        "deleted_message_excluded",
        count(find("will").as_ref()) == Some(0),
        "deleted",
    );
    evidence.check(
        "hidden_attachment_filename_excluded",
        count(find("Hidden").as_ref()) == Some(0),
        "hidden",
    );
    Ok(())
}
