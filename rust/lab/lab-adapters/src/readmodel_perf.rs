use crate::{demo, http::Client, journey::GATEWAY_SCOPE};
use lab_demo::{Options, generate};
use lab_runner::Evidence;
use lab_world::{Result, World};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const SCOPE: &str = "messages:read,system:read";
const CHECKS: [(&str, &str); 5] = [
    (
        "message_rows_and_payloads_match_canonical",
        "SELECT (SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0)=(SELECT count(*) FROM msg_by_chat) AND NOT EXISTS(SELECT 1 FROM msg_by_chat m LEFT JOIN records r ON r.collection='messages.message' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)",
    ),
    (
        "reaction_rows_and_payloads_match_canonical",
        "SELECT (SELECT count(*) FROM records WHERE collection='messages.reaction' AND deleted=0)=(SELECT count(*) FROM reaction_by_message) AND NOT EXISTS(SELECT 1 FROM reaction_by_message m LEFT JOIN records r ON r.collection='messages.reaction' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)",
    ),
    (
        "attachment_rows_and_payloads_match_canonical",
        "SELECT (SELECT count(*) FROM records WHERE collection='messages.attachment' AND deleted=0)=(SELECT count(*) FROM attachment_by_message) AND NOT EXISTS(SELECT 1 FROM attachment_by_message m LEFT JOIN records r ON r.collection='messages.attachment' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)",
    ),
    (
        "chat_summaries_match_index_aggregates",
        "SELECT NOT EXISTS(SELECT 1 FROM chat_summary s WHERE s.unread_count!=coalesce((SELECT sum(m.unread) FROM msg_by_chat m WHERE m.chat_id=s.id),0) OR s.last_activity_at!=coalesce((SELECT at FROM activity_by_chat a WHERE a.chat_id=s.id ORDER BY at DESC,id DESC LIMIT 1),''))",
    ),
    (
        "activity_rows_match_messages_and_reactions",
        "SELECT (SELECT count(*) FROM activity_by_chat)=(SELECT count(*) FROM msg_by_chat)+(SELECT count(*) FROM reaction_by_message r JOIN msg_by_chat m ON m.id=r.message_id) AND NOT EXISTS(SELECT 1 FROM activity_by_chat a LEFT JOIN msg_by_chat m ON m.id=a.message_id WHERE m.id IS NULL OR m.chat_id!=a.chat_id OR (a.id=a.message_id AND a.at!=m.sent_at) OR (a.id!=a.message_id AND NOT EXISTS(SELECT 1 FROM reaction_by_message r WHERE r.id=a.id AND r.message_id=a.message_id AND r.at=a.at)))",
    ),
];

fn open(world: &World) -> Option<Connection> {
    Connection::open_with_flags(
        world.gateway_home.join("state.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()
}
fn targets(db: &Connection) -> Vec<(String, Value)> {
    let Ok(mut statement) = db.prepare(
        "SELECT a.native_key,r.payload FROM records r JOIN aliases a ON a.record_id=r.id AND a.collection=r.collection WHERE r.collection='messages.message' AND r.deleted=0 ORDER BY r.id LIMIT 100",
    ) else {
        return vec![];
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    }) else {
        return vec![];
    };
    rows.filter_map(std::result::Result::ok)
        .filter_map(|(key, payload)| Some((key, serde_json::from_slice(&payload).ok()?)))
        .collect()
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let started = Instant::now();
    let seeded = demo::load(
        world,
        &generate(Options {
            seed: 1,
            huge: true,
        }),
        900,
    )
    .is_ok();
    let elapsed = started.elapsed();
    evidence.check(
        "huge_demo_seed_within_dataset_120_second_budget",
        seeded && elapsed < Duration::from_secs(120),
        &format!("{} ms", elapsed.as_millis()),
    );
    if !seeded {
        return Ok(());
    }
    evidence.check(
        "synthetic_fixture_generation",
        true,
        "synthetic huge demo dataset",
    );
    let gateway = Client::new(world, SCOPE)?;
    let ready = crate::assertions::await_until(300, || {
        gateway
            .call_raw(
                "search",
                &json!({"text": "readmodelDefinitelyAbsent", "limit": 1}),
                30,
            )
            .is_ok_and(|reply| reply.is_some_and(|(status, _)| status < 300))
    });
    if !ready {
        evidence.check("search_ready", false, "search index never became ready");
        return Ok(());
    }
    let Some(db) = open(world) else {
        evidence.check("inspection_connection", false, "state.db unavailable");
        return Ok(());
    };
    let messages: i64 = db
        .query_row(
            "SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    evidence.check(
        "fixed_104900_message_fixture",
        messages == 104_900,
        &format!("{messages} persisted messages"),
    );
    if messages != 104_900 {
        return Ok(());
    }
    let chosen = targets(&db);
    evidence.check(
        "exactly_100_existing_update_targets",
        chosen.len() == 100,
        "",
    );
    if chosen.len() != 100 {
        return Ok(());
    }
    let admin = Client::new(world, GATEWAY_SCOPE)?;
    let generation = world.control("/messages-status", &Value::Null)?["generation"].clone();
    let mut predecessor = world.control("/stats", &Value::Null)?["checkpoints"]["messages"].clone();
    let mut timings = vec![];
    for round in 0..5 {
        let observations: Vec<Value> = chosen
            .iter()
            .map(|(key, record)| {
                let mut data = record["data"].clone();
                data["text"] = json!(format!("[ab-test] readmodel round {round}"));
                data["read_at"] = if round % 2 == 0 {
                    json!("2030-01-01T00:00:00Z")
                } else {
                    Value::Null
                };
                json!({"kind": "upsert", "collection": record["collection"], "native_key": key, "observed_at": format!("2030-01-01T00:00:0{round}Z"), "data": data})
            })
            .collect();
        let before = admin.cursor()?.sequence;
        let checkpoint = json!(format!("readmodel-perf-{round}"));
        let begin = Instant::now();
        let sent = world
            .control(
                "/capture-exact",
                &json!({"source": "messages", "generation": generation, "predecessor": predecessor, "checkpoint": checkpoint, "observations": observations}),
            )
            .is_ok();
        predecessor = checkpoint;
        timings.push(begin.elapsed().as_secs_f64() * 1000.0);
        let after = admin.cursor()?.sequence;
        evidence.check(
            "capture_commits_exactly_100_changes",
            sent && after.saturating_sub(before) == 100,
            &format!("{} changes", after.saturating_sub(before)),
        );
        if !sent {
            return Ok(());
        }
    }
    timings.sort_by(f64::total_cmp);
    let p95 = timings.last().copied().unwrap_or(f64::MAX);
    evidence.check(
        "capture_commit_p95_under_one_second",
        p95 < 1000.0,
        &format!("{p95:.1} ms"),
    );
    for (name, query) in CHECKS {
        let correct = open(world)
            .and_then(|db| db.query_row(query, [], |row| row.get::<_, bool>(0)).ok())
            .unwrap_or(false);
        evidence.check(name, correct, "");
    }
    Ok(())
}
