#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{
        Observed, await_until, converged, false_deletion_free, read_all, record_integrity,
        replica_integrity, truth_matches,
    },
    history::SCOPE,
    http::{Client, random_id},
    replicas,
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const SCENARIOS: [&str; 3] = ["CON-IMPORT", "NOTE-IMPORT", "USE-IMPORT"];

fn guard(evidence: &Evidence, expected: &str, action: &str) -> Result<()> {
    if evidence.scenario == expected {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "{action} not ported for {}",
            evidence.scenario
        )))
    }
}
fn tag(prefix: &str) -> Result<String> {
    Ok(format!("[ab-test {}]", random_id(prefix)?))
}
fn records_of(client: &Client, collection: &str) -> Vec<Value> {
    read_all(client, false, false)
        .map(|read| {
            read.records
                .into_iter()
                .filter(|record| record["collection"] == collection)
                .collect()
        })
        .unwrap_or_default()
}
fn observers(observed: &Observed) -> Result<Vec<Watcher>> {
    observed
        .clients()
        .iter()
        .map(|client| Watcher::start(client, false))
        .collect()
}
fn stream_checks(
    world: &World,
    observed: &Observed,
    watchers: &[Watcher],
    evidence: &mut Evidence,
    contiguous: &str,
    deletion: &str,
) {
    for (index, watcher) in watchers.iter().enumerate() {
        evidence.check(
            &format!("{contiguous}_{index}"),
            watcher.valid(),
            "every batch continued the previous watermark",
        );
        evidence.check(
            &format!("{deletion}_{index}"),
            false_deletion_free(world, observed, watcher),
            "every observed delete is absent in SimApple truth",
        );
    }
}
pub fn contacts(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "CON-IMPORT", "contacts")?;
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let tag = tag("contacts")?;
    let name = format!("{tag} Testperson");
    let synthetic = json!({
        "given_name": name, "family_name": "Testfamily", "organization": "Bridge Test",
        "job_title": "Testjob",
        "phones": [{"label": "mobile", "value": "+15555550123"}],
        "emails": [{"label": "home", "value": "bridge-test@example.com"}]
    });
    let find = || {
        records_of(&gateway, "contacts.contact")
            .into_iter()
            .find(|record| {
                record["data"]["given_name"] == name.as_str()
                    && record["data"]["organization"] == "Bridge Test"
            })
    };
    let control = |input: Value| world.control("/contacts", &input).is_ok();
    if !control(json!({"action":"create","key":tag,"data":synthetic})) {
        evidence.check("fixture_create", false, "contact fixture rejected");
        return Ok(());
    }
    let imported = await_until(15, || find().is_some());
    evidence.check(
        "unified_contacts_imported",
        imported,
        "contact imported by the gateway",
    );
    if !imported {
        return Ok(());
    }
    let watchers = observers(&observed)?;
    if !control(json!({"action":"group","key":tag,"data":{}})) {
        evidence.check("group_fixture", false, "group fixture rejected");
        return Ok(());
    }
    if !control(
        json!({"action":"create","key":"linked-test-native","unified_key":tag,
        "data":{"emails":[{"label":"work","value":"linked-test@example.com"}]}}),
    ) {
        evidence.check("linked_native_fixture", false, "linked fixture rejected");
        return Ok(());
    }
    let unified = find().is_some_and(|record| {
        record["data"]["emails"]
            .as_array()
            .is_some_and(|emails| emails.len() == 2)
    });
    evidence.check(
        "linked_contacts_are_unified",
        unified,
        "native contacts with one unified key merge their emails",
    );
    evidence.check(
        "sim_truth_equal",
        await_until(5, || truth_matches(world, &observed)),
        "gateway contacts equal SimApple truth",
    );
    evidence.check(
        "replica_convergence",
        await_until(10, || converged(world, &observed)),
        "replica contact records equal the gateway",
    );
    evidence.check(
        "independent_sim_truth",
        await_until(5, || truth_matches(world, &observed)),
        "gateway contacts equal SimApple truth",
    );
    stream_checks(
        world,
        &observed,
        &watchers,
        evidence,
        "sse_contiguous",
        "no_false_deletion",
    );
    let token = world
        .gateway_cli(&["token", "create", "--scope", "system:read"])?
        .trim()
        .to_owned();
    let hidden = Client::at(gateway.address(), &token).call(
        "records.query",
        &json!({"collection":"contacts.contact","limit":1000}),
        20,
    );
    evidence.check(
        "contacts_scope_enforced",
        hidden.is_ok_and(|page| {
            page.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty))
        }),
        "system:read cannot read contacts",
    );
    record_integrity(evidence, &observed, None);
    Ok(())
}
pub fn notes(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "NOTE-IMPORT", "notes")?;
    let gateway = Client::new(world, SCOPE)?;
    let tag = tag("notes")?;
    let title = format!("{tag} note");
    let change = |key: &str, collection: &str, data: Value| {
        world
            .control(
                "/external-change",
                &json!({"key": key, "collection": collection, "data": data}),
            )
            .is_ok()
    };
    if !change(
        "note-folder",
        "notes.folder",
        json!({"title":"Bridge Test","account":"sim"}),
    ) {
        evidence.check("folder_fixture", false, "folder fixture rejected");
        return Ok(());
    }
    let data = json!({
        "title": title, "plain_text": format!("{title}\nsynthetic body\n"),
        "folder_id": {"$ref": {"collection": "notes.folder", "native_key": "note-folder"}},
        "locked": false, "editable": "plain", "content_status": "complete",
        "attachments": [], "html": null, "block_status": "complete", "round_trip_status": "unproven"
    });
    if !change("external-note", "notes.note", data) {
        evidence.check("native_fixture", false, "note fixture rejected");
        return Ok(());
    }
    let find = |wanted: &str| {
        records_of(&gateway, "notes.note")
            .into_iter()
            .find(|record| {
                record["data"]["title"] == wanted && record["data"]["recently_deleted"] != true
            })
    };
    let imported = await_until(30, || find(&title).is_some());
    evidence.check(
        "external_note_imported",
        imported,
        "external note imported by the gateway",
    );
    let Some(note) = find(&title) else {
        return Ok(());
    };
    let locked = json!({"title": format!("{tag} locked"), "plain_text": "", "locked": true,
        "editable": "none", "content_status": "locked", "attachments": []});
    let locked_title = format!("{tag} locked");
    evidence.check(
        "locked_title_only",
        change("locked-note", "notes.note", locked)
            && await_until(5, || {
                find(&locked_title).is_some_and(|record| {
                    record["data"]["plain_text"] == ""
                        && record["data"]["content_status"] == "locked"
                })
            }),
        "locked note keeps its title and no body",
    );
    let body = &note["data"];
    evidence.check(
        "folder_reference",
        !body["folder_id"].is_null() && body["folder_id"] != "",
        "note carries a folder reference",
    );
    evidence.check(
        "plain_body",
        body["plain_text"] == format!("{title}\nsynthetic body\n").as_str(),
        "note plain text body imported",
    );
    Ok(())
}
pub fn usage_totals(client: &Client, device: &str) -> (f64, f64) {
    let Ok(read) = read_all(client, false, true) else {
        return (-1.0, -1.0);
    };
    let ids: BTreeMap<&str, &str> = read
        .aliases
        .iter()
        .filter(|alias| alias["collection"] == "usage.device")
        .filter_map(|alias| Some((alias["record_id"].as_str()?, alias["native_key"].as_str()?)))
        .collect();
    let (mut seconds, mut clipped) = (0.0, 0.0);
    for record in &read.records {
        if record["collection"] != "usage.app_day" {
            continue;
        }
        let id = record["data"]["device_id"].as_str().unwrap_or_default();
        if ids.get(id) == Some(&device) {
            seconds += record["data"]["seconds_estimated"].as_f64().unwrap_or(0.0);
            clipped += record["data"]["clipped_seconds"].as_f64().unwrap_or(0.0);
        }
    }
    (seconds, clipped)
}
pub fn usage(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "USE-IMPORT", "usage")?;
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let base = 1_800_100_800.0_f64;
    let event = |offset: f64, starting: bool, bundle: &str| json!({"at": base + offset, "starting": starting, "bundle": bundle});
    let send = |device: &str, platform: &str, events: Vec<Value>| {
        world
            .control(
                "/usage",
                &json!({"device": device, "platform": platform, "events": events}),
            )
            .is_ok()
    };
    let app = "test.synthetic.alpha";
    if !send(
        "local",
        "mac",
        vec![
            event(0.0, true, app),
            event(600.0, true, "loginwindow"),
            event(3600.0, false, app),
            event(0.0, true, app),
            event(999.0, false, ""),
        ],
    ) {
        evidence.check("usage_fixture", false, "usage fixture rejected");
        return Ok(());
    }
    let clipped = await_until(10, || {
        let (seconds, clipped) = usage_totals(&gateway, "local");
        (seconds - 600.0).abs() < f64::EPSILON && (clipped - 3000.0).abs() < f64::EPSILON
    });
    evidence.check(
        "mac_lock_clipped",
        clipped,
        "lock time clipped from the focus session",
    );
    if !clipped {
        return Ok(());
    }
    evidence.check(
        "remote_import",
        send(
            "remote",
            "iphone",
            vec![event(0.0, true, app), event(120.0, false, app)],
        ),
        "remote device batch imported",
    );
    evidence.check(
        "per_device_totals",
        await_until(10, || {
            (usage_totals(&gateway, "remote").0 - 120.0).abs() < f64::EPSILON
        }),
        "remote totals reach the gateway",
    );
    usage_tail(&gateway, &observed, evidence)
}
pub fn usage_tail(gateway: &Client, observed: &Observed, evidence: &mut Evidence) -> Result<()> {
    evidence.check(
        "replica_converged",
        await_until(30, || {
            let usage_only = |client: &Client| {
                read_all(client, false, false).map(|read| {
                    read.records
                        .into_iter()
                        .filter(|r| {
                            r["collection"] == "usage.device" || r["collection"] == "usage.app_day"
                        })
                        .collect::<Vec<_>>()
                })
            };
            let gateway_records = usage_only(gateway);
            replicas::endpoints().iter().all(|endpoint| {
                match (&gateway_records, usage_only(&endpoint.client())) {
                    (Ok(left), Ok(right)) => {
                        !left.is_empty()
                            && crate::assertions::record_multiset(left, true)
                                == crate::assertions::record_multiset(&right, true)
                    }
                    _ => false,
                }
            }) && replica_integrity(observed)
        }),
        "replica usage records equal the gateway",
    );
    let read = read_all(gateway, false, false)?;
    let mut metadata = true;
    let (mut devices, mut days) = (0, 0);
    let mut names = std::collections::BTreeSet::new();
    let mut day_totals: BTreeMap<String, f64> = BTreeMap::new();
    let mut bounded = true;
    for record in &read.records {
        let data = &record["data"];
        match record["collection"].as_str().unwrap_or_default() {
            "usage.device" => {
                devices += 1;
                let name = data["name"].as_str().unwrap_or_default();
                metadata = metadata
                    && name != "Unnamed device"
                    && !name.is_empty()
                    && names.insert(name.to_owned())
                    && data["platform"].as_str().is_some_and(|p| !p.is_empty())
                    && !data["latest_event_at"].is_null();
            }
            "usage.app_day" => {
                days += 1;
                let key = format!(
                    "{}/{}",
                    data["device_id"].as_str().unwrap_or_default(),
                    data["day"].as_str().unwrap_or_default()
                );
                let seconds = data["seconds_estimated"].as_f64().unwrap_or(0.0);
                let total = day_totals.entry(key).or_insert(0.0);
                *total += seconds;
                bounded = bounded && seconds >= 0.0 && *total <= 86_400.001;
                metadata = metadata
                    && data["provenance"] == "biome.app_infocus"
                    && data["decoder_version"] == "infocus-segb-1"
                    && data["uncertainty"].as_str().is_some_and(|u| !u.is_empty());
            }
            _ => {}
        }
    }
    evidence.check(
        "daily_device_totals_physically_bounded",
        bounded,
        "no device-day exceeds 24 hours",
    );
    evidence.check(
        "provenance_and_device_freshness",
        metadata && devices > 0 && days > 0,
        "device metadata and provenance present",
    );
    Ok(())
}
