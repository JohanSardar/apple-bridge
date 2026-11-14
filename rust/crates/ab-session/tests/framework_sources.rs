mod support;

use ab_session::{production, readers};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use support::{Fallible, expect_silence, receive, send, temporary};

type Facts = Arc<Mutex<BTreeMap<String, Result<Vec<Value>, String>>>>;

fn serve_one(mut stream: UnixStream, facts: &Facts) -> Fallible<()> {
    loop {
        let (_, frame) = receive(&mut stream)?;
        if frame["t"] == "Hello" {
            send(
                &mut stream,
                &json!({"t":"Welcome","protocol":"native/1","version":1}),
            )?;
            continue;
        }
        let op = frame["op"].as_str().unwrap_or_default();
        let answer = facts
            .lock()
            .map_err(|_| "poisoned")?
            .get(op)
            .cloned()
            .unwrap_or_else(|| Err("unsupported".into()));
        let reply = match answer {
            Ok(items) => json!({
                "t":"FactPage","id":frame["id"],"op":op,"snapshot":"s","items":items,
                "cursor":null,"complete":true,"scope":"test","generation":"native/1"
            }),
            Err(code) => json!({"t":"Error","id":frame["id"],"code":code}),
        };
        send(&mut stream, &reply)?;
    }
}

fn fact_server(directory: &Path, facts: &Facts) -> Fallible<PathBuf> {
    let path = directory.join("facts.sock");
    let listener = UnixListener::bind(&path)?;
    let facts = Arc::clone(facts);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let facts = Arc::clone(&facts);
            thread::spawn(move || {
                let _ = serve_one(stream, &facts);
            });
        }
    });
    Ok(path)
}

fn set(facts: &Facts, op: &str, items: Vec<Value>) -> Fallible<()> {
    facts
        .lock()
        .map_err(|_| "poisoned")?
        .insert(op.to_owned(), Ok(items));
    Ok(())
}

fn keys(capture: &Value) -> Vec<(String, String)> {
    capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            (
                row["kind"].as_str().unwrap_or_default().to_owned(),
                row["native_key"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn focus_combines_the_framework_fact_with_the_assertions_file() -> Fallible<()> {
    let home = temporary("focus-home")?;
    let root = temporary("focus-root")?;
    let sockets = temporary("focus-sock")?;
    let database = home.join("Library/DoNotDisturb/DB");
    std::fs::create_dir_all(&database)?;
    let file = database.join("Assertions.json");
    let write = |on: bool| {
        let records = if on { "[{}]" } else { "[]" };
        std::fs::write(
            &file,
            format!(
                "{{\"header\":{{\"version\":8}},\"data\":[{{\"storeAssertionRecords\":{records}}}]}}"
            ),
        )
    };
    write(false)?;
    let facts: Facts = Facts::default();
    set(
        &facts,
        "focus.status",
        vec![json!({"authorization":"authorized","center_focused":false})],
    )?;
    let socket = fact_server(&sockets, &facts)?;
    let config = json!({"focus":{"root":root,"home":home,"facts":{"socket":socket},"facts_poll_ms":3_600_000}});
    let mut session = support::wire(
        readers::from_config(&config.to_string())?,
        production::Options {
            poll: Duration::from_millis(20),
            ..production::Options::default()
        },
        json!({}),
    )?;
    let (size, first) = receive(&mut session.gateway)?;
    let row = &first["observations"][0];
    assert_eq!(row["collection"], "focus.status");
    assert_eq!(row["scoped_alias"], "focus/status/current");
    assert_eq!(
        row["data"],
        json!({"focused":false,"name":null,"authorization":"authorized"})
    );
    session.commit("focus", &first, size)?;
    expect_silence(&mut session.gateway)?;

    write(true)?;
    let (size, second) = receive(&mut session.gateway)?;
    assert_eq!(second["observations"][0]["data"]["focused"], true);
    session.commit("focus", &second, size)?;
    session.finish()?;
    Ok(())
}

#[test]
fn pdf_artifacts_are_added_and_never_inferred_deleted() -> Fallible<()> {
    let root = temporary("pdf-root")?;
    let config = json!({"pdf":{"root":root}}).to_string();
    let mut reader = readers::from_config(&config)?.remove("pdf").ok_or("pdf")?;
    let context = |checkpoint: &str| readers::ScanContext {
        checkpoint: checkpoint.into(),
        scan_id: "s".into(),
        now: 1_800_000_000.0,
        fence: 7,
    };
    assert!(reader.next(&context("c0"))?.is_none());
    let add = |key: &str| json!({"native_key": key, "digest": "d1", "handle": "pdf:d1", "data": {"annotation_count": 1}});
    assert!(
        reader
            .answer("pdf.add_artifact", &add("one"))
            .ok_or("answer")?
            .is_ok()
    );
    let first = reader.next(&context("c1"))?.ok_or("first")?;
    assert_eq!(first["blob_handles"]["d1"], "pdf:d1");
    assert_eq!(keys(&first), [("upsert".to_owned(), "one".to_owned())]);
    reader.commit()?;
    assert!(reader.next(&context("c2"))?.is_none());
    assert!(
        reader
            .answer("pdf.add_artifact", &add("two"))
            .ok_or("answer")?
            .is_ok()
    );
    let second = reader.next(&context("c3"))?.ok_or("second")?;
    assert_eq!(keys(&second), [("upsert".to_owned(), "two".to_owned())]);
    reader.commit()?;
    assert!(reader.answer("pdf.other", &json!({})).is_none());
    assert!(
        reader
            .answer("pdf.add_artifact", &json!({}))
            .ok_or("answer")?
            .is_err()
    );
    Ok(())
}

fn loaded(home: &Path, user: &Path) -> Fallible<production::Loaded> {
    Ok(production::load(home, Some(user))?.ok_or("nothing loaded")?)
}

#[test]
fn without_a_switch_file_every_source_is_captured_from_the_users_home() -> Fallible<()> {
    let home = temporary("default-home")?;
    let user = temporary("default-user")?;
    let loaded = loaded(&home, &user)?;
    let mut sources: Vec<String> = loaded.sources().to_vec();
    sources.sort();
    let mut expected: Vec<String> = production::SUPPORTED
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    expected.sort();
    assert_eq!(sources, expected);
    assert!(loaded.wants_inherited_facts());
    Ok(())
}

#[test]
fn an_existing_switch_file_keeps_its_roots_and_gains_the_new_sources() -> Fallible<()> {
    let home = temporary("override-home")?;
    let user = temporary("override-user")?;
    let root = temporary("override-root")?;
    std::fs::create_dir_all(home.join("session"))?;
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"phone":{"root":root}},"poll_ms":77,"watch":false}).to_string(),
    )?;
    let loaded = loaded(&home, &user)?;
    assert_eq!(loaded.sources().len(), production::SUPPORTED.len());
    assert_eq!(loaded.options().poll, Duration::from_millis(77));
    assert!(!loaded.options().watch);
    Ok(())
}

#[test]
fn an_exclusive_file_limits_the_session_to_the_named_sources() -> Fallible<()> {
    let home = temporary("exclusive-home")?;
    let user = temporary("exclusive-user")?;
    let root = temporary("exclusive-root")?;
    std::fs::create_dir_all(home.join("session"))?;
    let write =
        |value: &Value| std::fs::write(home.join(production::CONFIG_FILE), value.to_string());
    write(&json!({"exclusive":true,"sources":{"phone":{"root":root},"focus":{}}}))?;
    let named = loaded(&home, &user)?;
    assert_eq!(named.sources(), ["focus", "phone"]);
    write(&json!({"exclusive":true,"sources":{}}))?;
    assert!(production::load(&home, Some(&user))?.is_none());
    Ok(())
}

#[test]
fn without_a_user_home_only_the_file_counts() -> Fallible<()> {
    let home = temporary("fixture-home")?;
    assert!(production::load(&home, None)?.is_none());
    Ok(())
}

#[test]
fn readiness_of_a_rust_decided_source_replaces_what_the_helper_said() -> Fallible<()> {
    let root = temporary("ready-root")?;
    let sessions = temporary("ready-sessions")?;
    let staging = temporary("ready-staging")?;
    let config = json!({"voicemail":{"root":root,"staging":staging.join("voicemail")},"voice_memos":{"root":sessions,"staging":staging.join("voice_memos")}});
    let started = Instant::now();
    let mut session = support::wire(
        readers::from_config(&config.to_string())?,
        production::Options {
            poll: Duration::from_millis(20),
            readiness: true,
            ..production::Options::default()
        },
        json!({}),
    )?;
    let mut said = BTreeMap::new();
    while said.len() < 2 && started.elapsed() < Duration::from_secs(10) {
        let (_, frame) = receive(&mut session.gateway)?;
        if frame["t"] == "SourceState" {
            said.insert(
                frame["source"].as_str().unwrap_or_default().to_owned(),
                (frame["state"].clone(), frame["reason"].clone()),
            );
        }
    }
    assert_eq!(
        said.get("voicemail"),
        Some(&(json!("unavailable"), json!("store not present")))
    );
    assert_eq!(
        said.get("voice_memos"),
        Some(&(json!("unavailable"), json!("store not present")))
    );
    session.finish()?;
    Ok(())
}
