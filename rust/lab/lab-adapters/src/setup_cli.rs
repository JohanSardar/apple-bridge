use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    process::{Command, Output, Stdio},
    time::Instant,
};

const TITLE_MARKER: &str = "[ab-test]";

fn cli(world: &World, args: &[&str]) -> Result<Output> {
    Ok(Command::new(crate::replicas::binary()?)
        .args(args)
        .arg("--home")
        .arg(&world.gateway_home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdin(Stdio::null())
        .output()?)
}

fn events(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn source_total(stats: &Value, source: &str) -> i64 {
    stats["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["source"] == source)
        .and_then(|entry| entry["total"].as_i64())
        .unwrap_or(-1)
}

struct Seeded {
    count: u64,
    collection: String,
    source: String,
}

fn sync_and_watch(
    world: &World,
    input: &Value,
    seeded: &Seeded,
    evidence: &mut Evidence,
) -> Result<()> {
    let settle = input["settle"].as_u64().unwrap_or(2);
    let budget = input["budget"].as_u64().unwrap_or(60);
    let began = Instant::now();
    world.control_for(
        "/seed",
        &json!({"collection": seeded.collection, "count": seeded.count}),
        budget,
    )?;
    let watched = cli(
        world,
        &[
            "stats",
            "--watch",
            "--settle",
            &settle.to_string(),
            "--timeout",
            &budget.to_string(),
            "--json",
        ],
    )?;
    let lines = events(&watched.stdout);
    let finished = lines
        .iter()
        .find(|line| line["event"] == "finished" && line["source"] == seeded.source.as_str());
    evidence.check(
        "stats_watch_reports_the_source_finishing_with_its_count",
        finished.is_some_and(|line| {
            line["counts"][&seeded.collection].as_u64() == Some(seeded.count)
                && line["total"].as_u64() >= Some(seeded.count)
        }),
        &format!("{} seeded", seeded.count),
    );
    evidence.check(
        "stats_watch_ends_when_every_source_has_finished",
        lines
            .last()
            .is_some_and(|line| line["event"] == "all_finished"),
        "",
    );
    #[allow(clippy::cast_precision_loss)]
    let seconds = (began.elapsed().as_secs_f64() - settle as f64).max(0.001);
    #[allow(clippy::cast_precision_loss)]
    let rate = seeded.count as f64 / seconds;
    evidence.check(
        "first_sync_time_measured",
        true,
        &format!(
            "{} records in {seconds:.1} s, {rate:.0} records/s",
            seeded.count
        ),
    );
    evidence.check(
        "stats_watch_never_shows_content",
        !String::from_utf8_lossy(&watched.stdout).contains(TITLE_MARKER),
        "",
    );
    Ok(())
}

fn counts_and_reads(world: &World, seeded: &Seeded, evidence: &mut Evidence) -> Result<()> {
    let stats: Value = serde_json::from_slice(&cli(world, &["stats", "--json"])?.stdout)?;
    evidence.check(
        "stats_count_matches_the_truth_peer",
        i64::try_from(seeded.count)
            .is_ok_and(|count| source_total(&stats, &seeded.source) >= count),
        "",
    );
    let human = cli(world, &["stats"])?;
    evidence.check(
        "stats_never_show_content",
        human.status.success() && !String::from_utf8_lossy(&human.stdout).contains(TITLE_MARKER),
        "",
    );
    if seeded.collection != "reminders.reminder" {
        return Ok(());
    }
    let read = cli(world, &["read", "reminders", "--json"])?;
    let answer: Value = serde_json::from_slice(&read.stdout)?;
    evidence.check(
        "read_returns_what_was_captured",
        read.status.success() && answer["count"].as_u64() == Some(seeded.count),
        &format!("count {}", answer["count"]),
    );
    let people = cli(world, &["read", "reminders", "--limit", "1"])?;
    let shown = String::from_utf8_lossy(&people.stdout);
    evidence.check(
        "read_prints_for_people_and_respects_the_limit",
        people.status.success() && shown.contains(TITLE_MARKER) && shown.contains("more"),
        "",
    );
    Ok(())
}

fn existing_gateway_is_respected(world: &World, evidence: &mut Evidence) -> Result<()> {
    let up = cli(world, &["up", "--json"])?;
    let answer: Value = serde_json::from_slice(&up.stdout)?;
    evidence.check(
        "up_finds_the_running_gateway_and_keeps_remote_off",
        up.status.success() && answer["started"] == false && answer["remote"] == "off",
        "",
    );
    let down = cli(world, &["down"])?;
    evidence.check(
        "down_leaves_a_gateway_it_did_not_start",
        down.status.code() == Some(1) && world.query("sources.status", &json!({})).is_ok(),
        "",
    );
    let doctor = cli(world, &["doctor"])?;
    evidence.check(
        "existing_commands_still_answer",
        doctor.status.success(),
        "",
    );
    Ok(())
}

pub fn run(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let collection = input["collection"].as_str().unwrap_or("reminders.reminder");
    let seeded = Seeded {
        count: input["count"].as_u64().unwrap_or(25),
        collection: collection.to_owned(),
        source: collection.split('.').next().unwrap_or_default().to_owned(),
    };
    sync_and_watch(world, input, &seeded, evidence)?;
    counts_and_reads(world, &seeded, evidence)?;
    existing_gateway_is_respected(world, evidence)
}
