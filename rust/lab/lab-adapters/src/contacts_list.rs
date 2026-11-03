#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{
        Visibility, await_until, gateway_aliases, read_all, record_multiset, truth_records,
    },
    http::Client,
    replicas::{self, Endpoint},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};

pub const SCENARIOS: [&str; 1] = ["WEB-CON-LIST"];
pub const SCOPE: &str = "contacts:read,messages:read,system:read";

fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}
fn call(client: &Client, mcp: bool, input: &Value) -> Option<Value> {
    if mcp {
        client.call_mcp("contacts.list", input, 20)
    } else {
        client.call("contacts.list", input, 20)
    }
    .ok()
    .flatten()
}
fn truth(world: &World, gateway: &Client) -> bool {
    let Ok(inspected) = world.control("/inspect", &Value::Null) else {
        return false;
    };
    let kept: Vec<Value> = inspected
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            let source = row["collection"]
                .as_str()
                .and_then(|c| c.split('.').next())
                .unwrap_or_default();
            source == "contacts" || source == "messages"
        })
        .cloned()
        .collect();
    let (Ok(aliases), Ok(read)) = (gateway_aliases(gateway), read_all(gateway, false, false))
    else {
        return false;
    };
    let Ok(expected) = truth_records(
        &Value::Array(kept),
        &aliases,
        &Visibility::from_scope(SCOPE),
    ) else {
        return false;
    };
    let mut actual = read.records;
    for record in &mut actual {
        if record["collection"] == "messages.handle"
            && let Some(fields) = record["data"].as_object_mut()
        {
            fields.remove("contact_id");
        }
    }
    record_multiset(&actual, false) == record_multiset(&expected, false)
}
fn converged(world: &World, gateway: &Client, replicas: &[Endpoint]) -> bool {
    let Ok(head) = gateway.cursor() else {
        return false;
    };
    truth(world, gateway)
        && replicas.iter().all(|endpoint| {
            endpoint
                .client()
                .cursor()
                .is_ok_and(|cursor| cursor == head)
        })
}
fn sections(value: &Value) -> Vec<Value> {
    value["sections"].as_array().cloned().unwrap_or_default()
}
fn contacts_of(section: &Value) -> Vec<Value> {
    section["contacts"].as_array().cloned().unwrap_or_default()
}

pub fn readmodels(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if evidence.scenario != "WEB-CON-LIST" {
        return Err(Error::Invalid(format!(
            "readmodels not ported for {}",
            evidence.scenario
        )));
    }
    evidence.check("world_start", true, "world and replica started");
    let digest = "b".repeat(64);
    let gateway = Client::new(world, SCOPE)?;
    let replica_list = replicas::endpoints();
    let fixtures = vec![
        (
            "contacts.contact",
            "con-list-alpha",
            json!({"given_name": "Zoe", "family_name": "Adams", "is_me": true, "organization": "Bridge Test", "emails": [{"label": "work", "value": "con-list@example.com"}], "thumbnail_image": {"digest": digest, "state": "present"}}),
        ),
        (
            "contacts.contact",
            "con-list-beta",
            json!({"given_name": "Amy", "family_name": "Zulu", "organization": "Bridge Test"}),
        ),
        (
            "contacts.contact",
            "con-list-number",
            json!({"organization": "123 Test", "contact_type": "organization"}),
        ),
        (
            "contacts.group",
            "con-list-group",
            json!({"name": "Bridge Test List", "member_ids": [reference("contacts.contact", "con-list-alpha")]}),
        ),
        (
            "messages.handle",
            "con-list-handle",
            json!({"address": "con-list@example.com", "service": "iMessage"}),
        ),
        (
            "messages.chat",
            "con-list-chat",
            json!({"display_name": "Bridge Test Conversation", "participants": [reference("messages.handle", "con-list-handle")]}),
        ),
    ];
    let mut captured = true;
    for (collection, key, data) in &fixtures {
        captured &= world
            .control(
                "/external-change",
                &json!({"collection": collection, "key": key, "data": data}),
            )
            .is_ok();
    }
    captured &= world.control("/flush", &json!({})).is_ok();
    evidence.check("fixture_capture", captured, "fixtures captured");
    evidence.check(
        "replica_converged",
        await_until(10, || converged(world, &gateway, &replica_list)),
        "gateway records equal truth and replica cursors equal the gateway",
    );
    let aliases = gateway_aliases(&gateway).unwrap_or_default();
    let alias = |source: &str, collection: &str, key: &str| {
        aliases
            .get(&format!("{source}/{collection}/{key}"))
            .cloned()
            .unwrap_or_default()
    };
    let alpha = alias("contacts", "contacts.contact", "con-list-alpha");
    let group = alias("contacts", "contacts.group", "con-list-group");
    let chat = alias("messages", "messages.chat", "con-list-chat");
    evidence.check(
        "fixture_ids",
        !alpha.is_empty() && !group.is_empty() && !chat.is_empty(),
        "fixture aliases resolved",
    );
    let mut endpoints = vec![(
        gateway.clone(),
        world
            .gateway_cli(&["token", "create", "--scope", "contacts:read"])?
            .trim()
            .to_owned(),
        world
            .gateway_cli(&["token", "create", "--scope", "system:read"])?
            .trim()
            .to_owned(),
    )];
    for endpoint in &replica_list {
        endpoints.push((
            endpoint.client(),
            replicas::cli(
                &endpoint.home,
                &["token", "create", "--scope", "contacts:read"],
            )?,
            replicas::cli(
                &endpoint.home,
                &["token", "create", "--scope", "system:read"],
            )?,
        ));
    }
    for (index, (client, contacts_token, system_token)) in endpoints.iter().enumerate() {
        for protocol in ["rest", "mcp"] {
            let mcp = protocol == "mcp";
            let prefix = format!("{protocol}_{index}");
            let name = |suffix: &str| format!("{prefix}_{suffix}");
            let address = client.address();
            let scoped = |token: &str| Client::at(address, token);
            let out = call(
                client,
                mcp,
                &json!({"group": group, "sort": "family", "display_order": "given"}),
            );
            evidence.check(&name("available"), out.is_some(), "contacts.list");
            if let Some(out) = out {
                let list = sections(&out);
                let valid =
                    list.len() == 1 && list[0]["key"] == "A" && contacts_of(&list[0]).len() == 1;
                evidence.check(&name("membership_sections"), valid, "group membership");
                if valid {
                    let found = &contacts_of(&list[0])[0];
                    evidence.check(
                        &name("name_identity_photo"),
                        found["contact"]["id"] == alpha.as_str()
                            && found["display_name"] == "Zoe Adams"
                            && found["is_me"] == true
                            && found["avatar"]["digest"] == digest.as_str(),
                        "name identity photo",
                    );
                    let conversations = found["conversation_ids"].as_array().cloned();
                    evidence.check(
                        &name("linked_conversation"),
                        conversations.is_some_and(|ids| ids.len() == 1 && ids[0] == chat.as_str()),
                        "linked conversation",
                    );
                }
                let counted = out["groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|g| g["group"]["id"] == group.as_str() && g["count"] == 1);
                evidence.check(&name("group_count"), counted, "group count");
            } else {
                continue;
            }
            let device = call(
                client,
                mcp,
                &json!({"sort": "given", "display_order": "family"}),
            );
            let found = device.as_ref().is_some_and(|out| {
                sections(out).iter().any(|section| {
                    contacts_of(section).iter().any(|c| {
                        c["contact"]["id"] == alpha.as_str()
                            && section["key"] == "Z"
                            && c["display_name"] == "Adams Zoe"
                    })
                })
            });
            evidence.check(&name("device_preferences"), found, "device preferences");
            evidence.check(
                &name("invalid_sort"),
                call(client, mcp, &json!({"sort": "unsupported"})).is_none(),
                "invalid sort rejected",
            );
            evidence.check(
                &name("unknown_group"),
                call(client, mcp, &json!({"group": "missing"})).is_none(),
                "unknown group rejected",
            );
            let restricted = call(&scoped(contacts_token), mcp, &json!({"group": group}));
            let private = restricted.as_ref().is_some_and(|out| {
                sections(out).iter().all(|section| {
                    contacts_of(section)
                        .iter()
                        .all(|c| c["conversation_ids"].as_array().is_none_or(Vec::is_empty))
                })
            });
            evidence.check(&name("conversation_scope"), private, "conversation scope");
            let bare = call(&scoped(system_token), mcp, &json!({}));
            evidence.check(
                &name("contact_scope"),
                bare.is_some_and(|out| out["sections"] == json!([]) && out["groups"] == json!([])),
                "contact scope",
            );
        }
    }
    let deleted = world
        .control(
            "/external-change",
            &json!({"collection": "contacts.contact", "key": "con-list-alpha", "delete": true}),
        )
        .is_ok()
        && world.control("/flush", &json!({})).is_ok();
    evidence.check("deleted_contact_fixture", deleted, "contact deleted");
    for (client, _, _) in &endpoints {
        evidence.check(
            "deleted_members_do_not_count",
            await_until(10, || {
                call(client, false, &json!({"group": group})).is_some_and(|out| {
                    out["count"].as_u64().unwrap_or(0) == 0
                        && sections(&out).is_empty()
                        && out["groups"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|g| g["group"]["id"] == group.as_str() && g["count"] == 0)
                })
            }),
            "deleted members not counted",
        );
    }
    Ok(())
}
