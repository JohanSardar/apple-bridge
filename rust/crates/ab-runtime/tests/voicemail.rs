use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn bridge(home: &Path, arguments: &[&str]) -> Check<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_bridge"))
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .args(arguments)
        .arg("--home")
        .arg(home)
        .output()?)
}
struct Running<'a>(&'a Path);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        let _ = bridge(self.0, &["down"]);
    }
}

fn seed(directory: &Path) -> Check<String> {
    let store = directory.join("store");
    std::fs::create_dir(&store)?;
    let database = rusqlite::Connection::open(store.join("FaceTimeMessageStore-local.sqlitedb"))?;
    database.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB,ZFILETYPE TEXT);")?;
    let vectors: Value = serde_json::from_str(include_str!(
        "../../ab-native/tests/fixtures/voicemail-transcripts.json"
    ))?;
    let archive = data_encoding::BASE64
        .decode(vectors[0]["archive"].as_str().ok_or("archive")?.as_bytes())?;
    database.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic sender',10,2,0,?1,'m4a')", [archive])?;
    drop(database);
    let path = store.join("Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a");
    std::fs::create_dir_all(path.parent().ok_or("parent")?)?;
    let audio = b"synthetic recording bytes";
    std::fs::write(path, audio)?;
    let staging = directory.join("staging");
    let mut readers = ab_session::readers::from_config(
        &json!({"voicemail":{"root":store,"staging":staging}}).to_string(),
    )?;
    let mut capture = readers
        .get_mut("voicemail")
        .ok_or("reader")?
        .next(&ab_session::readers::ScanContext {
            checkpoint: "one".into(),
            scan_id: "one".into(),
            now: 1_767_225_600.0,
            fence: 1,
        })?
        .ok_or("capture")?;
    capture.as_object_mut().ok_or("object")?.remove("t");
    let capture: ab_wire::messages::Capture = serde_json::from_value(capture)?;
    let home = directory.join("home");
    {
        let mut state = ab_state::State::<ab_state::Gateway>::open(&home)?;
        state.commit_capture(&capture)?;
    }
    let (digest, handle) = capture.blob_handles.first_key_value().ok_or("blob")?;
    let handle: String = serde_json::from_str(handle.get())?;
    let chunk = readers
        .get_mut("voicemail")
        .ok_or("reader")?
        .blob(&handle, 0, 131_072)?;
    let fetched =
        data_encoding::BASE64.decode(chunk["bytes"].as_str().ok_or("chunk")?.as_bytes())?;
    assert_eq!(fetched, audio);
    {
        let mut blobs = ab_blobs::Store::open(&home, 1_000_000)?;
        let digest = ab_blobs::Digest::try_from(digest.clone())?;
        blobs.begin(&digest, u64::try_from(fetched.len())?)?;
        blobs.chunk(&digest, 0, &fetched)?;
        blobs.finish(&digest)?;
    }
    Ok(digest.clone())
}

#[tokio::test]
async fn voicemail_reader_reaches_cli_mcp_and_authorized_audio_downloads() -> Check {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let digest = seed(directory.path())?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    drop(listener);
    let started = bridge(
        &home,
        &[
            "up",
            "--api-addr",
            &address,
            "--peer-addr",
            "127.0.0.1:0",
            "--json",
        ],
    )?;
    let _running = Running(&home);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let output = bridge(&home, &["read", "voicemail", "--json"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        answer["records"][0]["data"]["transcript"]["text"],
        "Synthetic voicemail"
    );
    assert_eq!(answer["records"][0]["data"]["audio"]["digest"], digest);
    let output = bridge(&home, &["read", "voicemail"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("Synthetic voicemail"));
    assert!(text.contains(&digest));
    let token = bridge(&home, &["token", "create", "--scope", "voicemail:read"])?;
    assert!(token.status.success());
    let token = String::from_utf8(token.stdout)?;
    let client = reqwest::Client::builder().no_proxy().build()?;
    let mcp = |body: Value| {
        client
            .post(format!("http://{address}/mcp"))
            .bearer_auth(token.trim())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .body(body.to_string())
    };
    let initialized = mcp(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"voicemail-test","version":"1"}}})).send().await?;
    assert!(initialized.status().is_success());
    let response = mcp(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"records_query","arguments":{"collection":"voicemail.message"}}})).send().await?;
    assert!(response.status().is_success());
    let response: Value = serde_json::from_slice(&response.bytes().await?)?;
    assert_ne!(response["result"]["isError"], true, "{response}");
    let record = &response["result"]["structuredContent"]["records"][0];
    assert_eq!(record["data"], answer["records"][0]["data"]);
    let record_id = record["id"].as_str().ok_or("record id")?;
    let recording = client
        .get(format!("http://{address}/v1/blobs/{digest}"))
        .query(&[("record", record_id)])
        .bearer_auth(token.trim())
        .send()
        .await?;
    assert!(recording.status().is_success());
    assert_eq!(&recording.bytes().await?[..], b"synthetic recording bytes");
    let denied = client
        .get(format!("http://{address}/v1/blobs/{digest}"))
        .send()
        .await?;
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);
    let unrelated = bridge(&home, &["token", "create", "--scope", "contacts:read"])?;
    assert!(unrelated.status.success());
    let unrelated = String::from_utf8(unrelated.stdout)?;
    let denied = client
        .get(format!("http://{address}/v1/blobs/{digest}"))
        .query(&[("record", record_id)])
        .bearer_auth(unrelated.trim())
        .send()
        .await?;
    assert!(!denied.status().is_success());
    Ok(())
}
