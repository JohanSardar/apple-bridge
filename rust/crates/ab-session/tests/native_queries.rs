mod support;

use ab_session::{
    production::Options,
    queries::{MailBodies, NativeQueries},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::Arc,
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn send(socket: &mut UnixStream, value: &Value) -> Fallible<()> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

fn receive(socket: &mut UnixStream) -> Fallible<Value> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let mut bytes = vec![0; usize::try_from(u32::from_be_bytes(header))?];
    socket.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn emlx(directory: &Path, id: &str, message: &str) -> Fallible<()> {
    let folder = directory.join("Mailboxes/INBOX.mbox/Data/Messages");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(
        folder.join(format!("{id}.emlx")),
        format!("{}\n{message}", message.len()),
    )?;
    Ok(())
}

struct Session {
    gateway: UnixStream,
    helper: UnixStream,
    root: PathBuf,
}

fn session(bodies: bool, shortcuts: bool) -> Fallible<Session> {
    let root = support::temporary("session-mail")?;
    let sources = BTreeMap::new();
    let options = Options {
        queries: NativeQueries {
            mail_bodies: bodies.then(|| Arc::new(MailBodies::new(root.clone()))),
            shortcut_roots: shortcuts.then(|| vec![root.clone()]),
            ..NativeQueries::default()
        },
        ..Options::default()
    };
    let support::Wire {
        gateway,
        link: helper,
        worker: _,
        home: _,
    } = support::wire(sources, options, json!({}))?;
    Ok(Session {
        gateway,
        helper,
        root,
    })
}

fn query(id: &str, op: &str, args: &Value) -> Value {
    json!({"t":"Query","id":id,"op":op,"args":args,"deadline_ms":5000})
}

#[test]
fn rust_answers_mail_body_and_the_helper_never_sees_it() -> Fallible<()> {
    let mut s = session(true, false)?;
    emlx(&s.root, "42", "Content-Type: text/plain\n\nhello body\n")?;
    send(
        &mut s.gateway,
        &query("q1", "mail.body", &json!({"message_id":"42"})),
    )?;
    let reply = receive(&mut s.gateway)?;
    assert_eq!(reply["t"], "QueryResult");
    assert_eq!(reply["id"], "q1");
    assert_eq!(reply["blobs"], json!([]));
    assert_eq!(reply["result_json"]["state"], "downloaded");
    assert!(
        reply["result_json"]["plain_text"]
            .as_str()
            .is_some_and(|text| text.contains("hello body"))
    );

    send(
        &mut s.gateway,
        &query("q2", "mail.body", &json!({"message_id":"7"})),
    )?;
    let missing = receive(&mut s.gateway)?;
    assert_eq!(missing["result_json"]["state"], "offloaded");

    for (id, args) in [
        ("bad1", json!({"message_id":"abc"})),
        ("bad2", json!({"message_id":"42","extra":1})),
        ("bad3", json!({})),
        ("bad4", json!({"message_id":""})),
    ] {
        send(&mut s.gateway, &query(id, "mail.body", &args))?;
        let failure = receive(&mut s.gateway)?;
        assert_eq!(failure["t"], "QueryError", "{id}");
        assert_eq!(failure["code"], "invalid", "{id}");
    }

    send(&mut s.gateway, &query("other", "files.search", &json!({})))?;
    let forwarded = receive(&mut s.helper)?;
    assert_eq!(forwarded["id"], "other");
    std::fs::remove_dir_all(&s.root)?;
    Ok(())
}

#[test]
fn without_a_mail_owner_the_query_goes_to_the_helper() -> Fallible<()> {
    let mut s = session(false, false)?;
    send(
        &mut s.gateway,
        &query("q1", "mail.body", &json!({"message_id":"42"})),
    )?;
    let forwarded = receive(&mut s.helper)?;
    assert_eq!(forwarded["op"], "mail.body");
    std::fs::remove_dir_all(&s.root)?;
    Ok(())
}

#[test]
fn a_body_over_one_megabyte_travels_as_a_session_blob() -> Fallible<()> {
    let mut s = session(true, false)?;
    let message = format!("Content-Type: text/plain\n\n{}\n", "a".repeat(1_300_000));
    emlx(&s.root, "99", &message)?;
    send(
        &mut s.gateway,
        &query("big", "mail.body", &json!({"message_id":"99"})),
    )?;
    let reply = receive(&mut s.gateway)?;
    assert_eq!(reply["t"], "QueryResult");
    let blob = reply["blobs"][0].clone();
    let digest = blob["digest"].as_str().unwrap_or_default().to_owned();
    assert_eq!(reply["result_json"]["body_blob"], digest.as_str());
    let handle = blob["handle"].as_str().unwrap_or_default().to_owned();
    assert!(handle.starts_with("query-body:"));
    let size = blob["size"].as_u64().unwrap_or_default();
    let mut body = Vec::new();
    while u64::try_from(body.len())? < size {
        send(
            &mut s.gateway,
            &json!({"t":"BlobRead","handle":handle,"offset":body.len(),"length":131_072}),
        )?;
        let chunk = receive(&mut s.gateway)?;
        assert_eq!(chunk["t"], "BlobChunk");
        assert_eq!(chunk["digest"], digest.as_str());
        body.extend(STANDARD.decode(chunk["bytes"].as_str().unwrap_or_default())?);
        assert_eq!(chunk["eof"] == true, u64::try_from(body.len())? == size);
    }
    let hex = Sha256::digest(&body)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        });
    assert_eq!(hex, digest);
    let result: Value = serde_json::from_slice(&body)?;
    assert_eq!(result["state"], "downloaded");
    assert!(
        result["plain_text"]
            .as_str()
            .is_some_and(|t| t.len() >= 1_300_000)
    );
    send(
        &mut s.gateway,
        &json!({"t":"BlobRead","handle":handle,"offset":0,"length":16}),
    )?;
    let gone = receive(&mut s.gateway);
    assert!(
        gone.as_ref()
            .is_ok_and(|frame| frame["t"] == "BlobUnavailable"),
        "{gone:?}"
    );
    std::fs::remove_dir_all(&s.root)?;
    Ok(())
}

#[test]
fn rust_answers_shortcut_actions_and_the_helper_never_sees_them() -> Fallible<()> {
    let mut s = session(false, true)?;
    let metadata = s
        .root
        .join("Synthetic.app/Contents/Resources/Metadata.appintents");
    std::fs::create_dir_all(&metadata)?;
    std::fs::write(
        s.root.join("Synthetic.app/Contents/Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.synthetic</string></dict></plist>"#,
    )?;
    std::fs::write(
        metadata.join("extract.actionsdata"),
        r#"{"actions":{"Export":{"identifier":"Export","title":{"key":"Export"}}}}"#,
    )?;
    send(
        &mut s.gateway,
        &query("a1", "shortcuts.actions", &json!({})),
    )?;
    let reply = receive(&mut s.gateway)?;
    assert_eq!(reply["t"], "QueryResult");
    assert_eq!(reply["result_json"]["complete"], true);
    assert_eq!(
        reply["result_json"]["items"],
        json!([{"app":"com.example.synthetic","identifier":"Export","title":"Export"}])
    );
    send(
        &mut s.gateway,
        &query("a2", "shortcuts.actions", &json!({"x":1})),
    )?;
    let invalid = receive(&mut s.gateway)?;
    assert_eq!(invalid["t"], "QueryError");
    assert_eq!(invalid["code"], "invalid");
    send(
        &mut s.gateway,
        &query("m1", "mail.body", &json!({"message_id":"1"})),
    )?;
    let forwarded = receive(&mut s.helper)?;
    assert_eq!(forwarded["id"], "m1");
    std::fs::remove_dir_all(&s.root)?;
    Ok(())
}

#[test]
fn unrouted_shortcut_actions_go_to_the_helper() -> Fallible<()> {
    let mut s = session(false, false)?;
    send(
        &mut s.gateway,
        &query("a1", "shortcuts.actions", &json!({})),
    )?;
    let forwarded = receive(&mut s.helper)?;
    assert_eq!(forwarded["op"], "shortcuts.actions");
    std::fs::remove_dir_all(&s.root)?;
    Ok(())
}
