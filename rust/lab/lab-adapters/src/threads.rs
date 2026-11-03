#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    http::Client,
    readmodels::{call, observation, put, reference},
    watch::Watcher,
};
use lab_demo::{EPOCH, Options, add_date, generate, ts};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

const SCOPE: &str = "messages:read,contacts:read,system:read";

#[derive(Debug)]
struct Fixture {
    chat: String,
    other_chat: String,
    root: String,
    replies: Vec<String>,
}

fn message_data(extra: &Value, chat: &str, sent: i64, text: &str) -> Value {
    let mut data = json!({
        "chat_id": reference("messages.chat", chat),
        "sent_at": ts(sent),
        "kind": "message",
        "text": text,
        "parts": [{"index": 0, "kind": "text", "text": text}],
    });
    for (name, value) in extra.as_object().into_iter().flatten() {
        data[name] = value.clone();
    }
    data
}

fn aliases_of(page: &Value) -> impl Iterator<Item = (String, String)> + '_ {
    page["aliases"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|alias| {
            (
                alias["native_key"].as_str().unwrap_or_default().to_owned(),
                alias["record_id"].as_str().unwrap_or_default().to_owned(),
            )
        })
}

fn demo_ids(client: &Client) -> Result<BTreeMap<String, String>> {
    let mut ids = BTreeMap::new();
    for collection in ["messages.chat", "messages.handle", "messages.message"] {
        let mut after = String::new();
        let mut found = 0;
        loop {
            let page = client
                .call(
                    "records.query",
                    &json!({"collection": collection, "include_aliases": true, "after": after, "limit": 1000}),
                    60,
                )?
                .ok_or_else(|| Error::Invalid("records.query rejected".into()))?;
            for (key, id) in aliases_of(&page) {
                if collection == "messages.message" && !key.starts_with("demo-message-00-") {
                    continue;
                }
                if !ids.contains_key(&key) {
                    found += 1;
                }
                ids.insert(key, id);
            }
            let next = page["next"].as_str().unwrap_or_default();
            if next.is_empty() || (collection == "messages.message" && found == 200) {
                break;
            }
            next.clone_into(&mut after);
        }
    }
    if ids.len() == 300 {
        Ok(ids)
    } else {
        Err(Error::Invalid(format!(
            "demo fixtures incomplete: {} of 300",
            ids.len()
        )))
    }
}

fn populate(world: &World, huge: bool) -> Result<Fixture> {
    let mut dataset = generate(Options { seed: 1, huge });
    let root_at = add_date(EPOCH, 0, -12, 0);
    let mut records = vec![observation(
        "messages.message",
        "transcript-originator-root",
        &message_data(
            &json!({}),
            "demo-chat-39",
            root_at,
            "transcriptoriginatorfixture root",
        ),
    )];
    if !huge {
        for offset in -325_i64..=325 {
            if offset == 0 {
                continue;
            }
            records.push(observation(
                "messages.message",
                &format!("transcript-originator-gap-{:03}", offset + 325),
                &message_data(
                    &json!({}),
                    "demo-chat-39",
                    root_at + offset * 60,
                    "[ab-test transcript] bounded originator gap",
                ),
            ));
        }
    }
    for index in 0..205_i64 {
        records.push(observation(
            "messages.message",
            &format!("transcript-originator-reply-{index:03}"),
            &message_data(
                &json!({"thread_originator_id": reference("messages.message", "transcript-originator-root")}),
                "demo-chat-39",
                EPOCH + index,
                "transcriptoriginatorfixture distant reply",
            ),
        ));
    }
    for record in &mut records {
        record["observed_at"] = json!(ts(EPOCH));
    }
    dataset.records.extend(records);
    crate::demo::load(world, &dataset, 900)?;
    let gateway = Client::new(world, SCOPE)?;
    let ids = demo_ids(&gateway)?;
    let found = gateway
        .call(
            "records.query",
            &json!({"collection": "messages.message", "text": "transcriptoriginatorfixture", "include_aliases": true, "limit": 1000}),
            60,
        )?
        .ok_or_else(|| Error::Invalid("records.query rejected".into()))?;
    let aliases: BTreeMap<String, String> = aliases_of(&found).collect();
    let fixture = Fixture {
        chat: ids.get("demo-chat-39").cloned().unwrap_or_default(),
        other_chat: ids.get("message-chat").cloned().unwrap_or_default(),
        root: aliases
            .get("transcript-originator-root")
            .cloned()
            .unwrap_or_default(),
        replies: (0..205)
            .map(|i| {
                aliases
                    .get(&format!("transcript-originator-reply-{i:03}"))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect(),
    };
    if fixture.chat.is_empty()
        || fixture.root.is_empty()
        || found["records"].as_array().map_or(0, Vec::len) != 206
    {
        return Err(Error::Invalid("thread fixture incomplete".into()));
    }
    Ok(fixture)
}

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn count(page: &Value) -> usize {
    page["items"].as_array().map_or(0, Vec::len)
}

fn text<'a>(page: &'a Value, name: &str) -> &'a str {
    page[name].as_str().unwrap_or_default()
}

fn percentile95(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let index = ((values.len() as f64 * 0.95).ceil() as usize).max(1) - 1;
    values[index]
}

fn timed(client: &Client, protocol: &str, input: &Value, samples: &mut Vec<f64>) -> Option<Value> {
    let start = Instant::now();
    let reply = call(client, protocol, "messages.thread", input);
    samples.push(start.elapsed().as_secs_f64() * 1000.0);
    reply
}

fn read_checks(world: &World, fixture: &Fixture, evidence: &mut Evidence, budgets: bool) {
    let mut want = vec![fixture.root.clone()];
    want.extend(fixture.replies.clone());
    let filtered = |extra: Value| {
        let mut input = json!({"chat_id": fixture.chat, "thread_originator_id": fixture.root});
        for (name, value) in extra.as_object().into_iter().flatten() {
            input[name] = value.clone();
        }
        input
    };
    for protocol in ["rest", "mcp"] {
        let token = Client::new(world, SCOPE);
        evidence.check(&format!("{protocol}_read_token"), token.is_ok(), "token");
        let Ok(client) = token else { continue };
        let first = call(
            &client,
            protocol,
            "messages.thread",
            &filtered(json!({"anchor": fixture.root, "limit": 200})),
        );
        let first_ok = first.as_ref().is_some_and(|f| {
            ids(f) == want[..200]
                && f["items"][0]["reply_count"] == json!(205)
                && text(f, "before").is_empty()
                && !text(f, "after").is_empty()
        });
        evidence.check(
            &format!("{protocol}_root_and_first_199_distant_replies"),
            first_ok,
            "first page",
        );
        let Some(first) = first.filter(|f| count(f) == 200) else {
            continue;
        };
        let oldest = call(
            &client,
            protocol,
            "messages.thread",
            &filtered(json!({"anchor": "oldest", "limit": 200})),
        );
        evidence.check(
            &format!("{protocol}_filtered_oldest_anchor"),
            oldest.is_some_and(|o| {
                ids(&o) == want[..200]
                    && text(&o, "before").is_empty()
                    && !text(&o, "after").is_empty()
            }),
            "oldest",
        );
        let second = call(
            &client,
            protocol,
            "messages.thread",
            &filtered(json!({"after": first["after"], "limit": 200})),
        );
        let second_before = second
            .as_ref()
            .map(|s| text(s, "before").to_owned())
            .unwrap_or_default();
        evidence.check(
            &format!("{protocol}_bounded_forward_page"),
            second.is_some_and(|s| {
                ids(&s) == want[200..]
                    && !text(&s, "before").is_empty()
                    && text(&s, "after").is_empty()
            }),
            "forward",
        );
        let back = call(
            &client,
            protocol,
            "messages.thread",
            &filtered(json!({"before": second_before, "limit": 200})),
        );
        evidence.check(
            &format!("{protocol}_reverse_page_without_duplicates"),
            back.is_some_and(|b| ids(&b) == want[..200]),
            "reverse",
        );
        let rejected = |input: Value| call(&client, protocol, "messages.thread", &input).is_none();
        evidence.check(
            &format!("{protocol}_filtered_cursor_rejected_without_filter"),
            rejected(json!({"chat_id": fixture.chat, "after": first["after"]})),
            "cursor without filter",
        );
        evidence.check(
            &format!("{protocol}_filtered_cursor_rejected_for_other_root"),
            rejected(json!({"chat_id": fixture.chat, "thread_originator_id": fixture.replies[0], "after": first["after"]})),
            "cursor other root",
        );
        evidence.check(
            &format!("{protocol}_page_limit_200"),
            rejected(filtered(json!({"limit": 201}))),
            "limit",
        );
        evidence.check(
            &format!("{protocol}_root_in_other_chat_rejected"),
            rejected(json!({"chat_id": fixture.other_chat, "thread_originator_id": fixture.root})),
            "other chat",
        );
        evidence.check(
            &format!("{protocol}_missing_root_rejected"),
            rejected(
                json!({"chat_id": fixture.chat, "thread_originator_id": "missing-synthetic-root"}),
            ),
            "missing root",
        );
        let center = call(
            &client,
            protocol,
            "messages.thread",
            &json!({"chat_id": fixture.chat, "anchor": fixture.root, "limit": 100}),
        );
        let mut healthy = center.is_some();
        let center = center.unwrap_or_else(|| json!({"items": []}));
        let mut window = ids(&center);
        for direction in ["before", "after"] {
            let mut page = center.clone();
            for _ in 0..2 {
                let token = text(&page, direction).to_owned();
                if token.is_empty() {
                    break;
                }
                let next = call(
                    &client,
                    protocol,
                    "messages.thread",
                    &json!({"chat_id": fixture.chat, direction: token, "limit": 100}),
                );
                healthy = next.is_some();
                page = next.unwrap_or_else(|| json!({"items": []}));
                if !healthy {
                    break;
                }
                window.extend(ids(&page));
            }
        }
        evidence.check(
            &format!("{protocol}_replies_outside_current_500_item_window"),
            healthy
                && window.len() == 500
                && !window.contains(&fixture.replies[0])
                && !window.contains(&fixture.replies[204]),
            "window",
        );
        if budgets {
            budget_checks(world, fixture, evidence, protocol, &client);
        }
    }
    let denied = Client::new(world, "system:read");
    evidence.check("restricted_token", denied.is_ok(), "token");
    for protocol in ["rest", "mcp"] {
        let page = denied.as_ref().ok().and_then(|client| {
            call(
                client,
                protocol,
                "messages.thread",
                &filtered(json!({"limit": 200})),
            )
        });
        evidence.check(
            &format!("{protocol}_filtered_visibility"),
            page.is_some_and(|p| count(&p) == 0),
            "visibility",
        );
    }
}

fn budget_checks(
    world: &World,
    fixture: &Fixture,
    evidence: &mut Evidence,
    protocol: &str,
    client: &Client,
) {
    let query = json!({"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": fixture.root, "limit": 100});
    let valid_page = |page: &Option<Value>| {
        page.as_ref()
            .is_some_and(|p| count(p) == 100 && p["items"][0]["id"] == json!(fixture.root))
    };
    let mut batch = vec![];
    let mut batch_valid = true;
    for _ in 0..100 {
        let page = timed(client, protocol, &query, &mut batch);
        batch_valid &= valid_page(&page);
    }
    evidence.check(
        &format!("{protocol}_filtered_batch_results"),
        batch_valid,
        "batch",
    );
    let p95 = percentile95(&mut batch);
    evidence.check(
        &format!("{protocol}_filtered_batch_p95_10ms"),
        p95 <= 10.0,
        &format!("p95 {p95:.2} ms"),
    );
    let ab = Client::new(world, SCOPE);
    evidence.check(&format!("{protocol}_ab_token"), ab.is_ok(), "token");
    let Ok(ab) = ab else { return };
    let reference_client = Client::new(world, SCOPE);
    evidence.check(
        &format!("{protocol}_reference_token"),
        reference_client.is_ok(),
        "token",
    );
    let Ok(reference_client) = reference_client else {
        return;
    };
    let unfiltered = json!({"chat_id": fixture.chat, "anchor": fixture.root, "limit": 100});
    let (mut durations, mut reference_durations) = (vec![], vec![]);
    let (mut valid, mut reference_valid) = (true, true);
    for sample in 0..100 {
        for filtered in [sample % 2 == 0, sample % 2 != 0] {
            if filtered {
                let page = timed(&ab, protocol, &query, &mut durations);
                valid &= valid_page(&page);
            } else {
                let page = timed(
                    &reference_client,
                    protocol,
                    &unfiltered,
                    &mut reference_durations,
                );
                reference_valid &=
                    page.is_some_and(|p| count(&p) == 100 && ids(&p).contains(&fixture.root));
            }
        }
    }
    evidence.check(
        &format!("{protocol}_unfiltered_reference_results"),
        reference_valid,
        "reference",
    );
    evidence.check(
        &format!("{protocol}_100k_filtered_read_results"),
        valid,
        "filtered",
    );
    let p95 = percentile95(&mut durations);
    evidence.check(
        &format!("{protocol}_100k_filtered_read_p95_10ms"),
        p95 <= 10.0,
        &format!("p95 {p95:.2} ms"),
    );
}

struct Window {
    items: usize,
    older: usize,
    newer: usize,
    ids: Vec<String>,
    records: Vec<Value>,
}

fn window(client: &Client, fixture: &Fixture) -> Option<Window> {
    let result = call(
        client,
        "rest",
        "messages.thread",
        &json!({"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": "latest", "limit": 200}),
    )?;
    let mut view = Window {
        items: count(&result),
        older: 0,
        newer: 0,
        ids: ids(&result),
        records: result["items"].as_array().cloned().unwrap_or_default(),
    };
    if !text(&result, "before").is_empty() {
        let older = call(
            client,
            "rest",
            "messages.thread",
            &json!({"chat_id": fixture.chat, "thread_originator_id": fixture.root, "before": result["before"], "limit": 200}),
        )?;
        view.older = count(&older);
    }
    if !text(&result, "after").is_empty() {
        view.newer = 1;
    }
    Some(view)
}

fn live_checks(world: &World, fixture: &Fixture, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let observer = Watcher::start(&client, false);
    evidence.check(
        "thread_rest_observer_registered",
        observer.is_ok(),
        "observer",
    );
    let Ok(observer) = observer else {
        return Ok(());
    };
    let snapshot = window(&client, fixture);
    evidence.check(
        "filtered_snapshot_scoped_counts",
        snapshot.is_some_and(|s| s.items == 200 && s.older == 6 && s.newer == 0),
        "snapshot",
    );
    let put_message = |key: &str, body: &str, originator: bool| {
        let extra = if originator {
            json!({"thread_originator_id": reference("messages.message", "transcript-originator-root")})
        } else {
            json!({})
        };
        let mut data = message_data(&extra, "demo-chat-39", EPOCH + 3600, body);
        data["parts"] = json!([{"index": 0, "kind": "text", "text": body}]);
        put(world, &[observation("messages.message", key, &data)])
    };
    let seen = |body: &str| {
        await_until(5, || {
            observer
                .first_seen_record(|record| {
                    record["collection"] == "messages.message" && record["data"]["text"] == body
                })
                .is_some()
        })
    };
    let wrote = put_message(
        "transcript-originator-unrelated",
        "An unrelated synthetic message",
        false,
    );
    let arrived = wrote.is_ok() && seen("An unrelated synthetic message");
    let unrelated = window(&client, fixture);
    evidence.check(
        "unrelated_live_message_excluded",
        arrived && unrelated.is_some_and(|u| u.items == 200 && u.older == 6 && u.newer == 0),
        "unrelated",
    );
    let wrote = put_message(
        "transcript-originator-live",
        "transcriptoriginatorlivefixture",
        true,
    );
    let arrived = wrote.is_ok() && seen("transcriptoriginatorlivefixture");
    let inserted = window(&client, fixture);
    let inserted_ok = inserted.is_some_and(|i| {
        i.items == 200
            && i.older == 7
            && i.newer == 0
            && i.records.iter().any(|item| {
                item["data"]["text"] == "transcriptoriginatorlivefixture"
                    && item["data"]["thread_originator_id"] == json!(fixture.root)
            })
    });
    evidence.check(
        "distant_live_reply_inserted_with_filtered_counts",
        arrived && inserted_ok,
        "inserted",
    );
    let reassigned = put(
        world,
        &[observation(
            "messages.message",
            "transcript-originator-reply-204",
            &json!({"chat_id": reference("messages.chat", "demo-chat-39"), "sent_at": ts(EPOCH + 204), "kind": "message", "text": "[ab-test originator] reassigned"}),
        )],
    );
    let arrived = reassigned.is_ok() && seen("[ab-test originator] reassigned");
    let removed = window(&client, fixture);
    evidence.check(
        "reassigned_live_reply_removed",
        arrived
            && removed.is_some_and(|r| {
                r.items == 200
                    && r.older == 6
                    && r.newer == 0
                    && !r.ids.contains(&fixture.replies[204])
            }),
        "removed",
    );
    Ok(())
}

pub fn originator(world: &World, evidence: &mut Evidence) -> Result<()> {
    let budgets = evidence.scenario.ends_with("-PERF");
    evidence.check("world_start", true, "world");
    let fixture = populate(world, budgets);
    evidence.check(
        "canonical_demo_and_distant_thread_captured",
        fixture.is_ok(),
        "fixture",
    );
    let fixture = fixture?;
    read_checks(world, &fixture, evidence, budgets);
    live_checks(world, &fixture, evidence)
}
