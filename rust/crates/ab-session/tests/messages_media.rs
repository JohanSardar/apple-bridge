#![cfg(feature = "rust-capture")]

use ab_session::{native_facts::Client, readers, routed};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

const PHOTO: &[u8] = include_bytes!("fixtures/messages-media/photo.png");
const SWIFT: &str = include_str!("fixtures/messages-media/swift-record.json");

fn send(socket: &mut UnixStream, value: &Value) -> Fallible<()> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

fn receive(socket: &mut UnixStream) -> Fallible<(usize, Value)> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header))?;
    let mut bytes = vec![0; size];
    socket.read_exact(&mut bytes)?;
    Ok((size + 4, serde_json::from_slice(&bytes)?))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ready,
    Failed,
    Unsupported,
    Broken,
    Waiting,
}

struct World {
    mode: Mode,
    requests: Vec<Value>,
    root: PathBuf,
}

type Shared = Arc<Mutex<World>>;

fn swift() -> Fallible<Value> {
    Ok(serde_json::from_str(SWIFT)?)
}

fn materialize(root: &Path) -> Fallible<()> {
    let record = swift()?;
    let variants = record["variants"].as_object().ok_or("variants")?;
    for variant in record["fact"]["result"]["derivatives"]
        .as_object()
        .ok_or("derivatives")?
        .values()
    {
        let digest = variant["digest"].as_str().ok_or("digest")?;
        let target = root.join(variant["file"].as_str().ok_or("file")?);
        std::fs::create_dir_all(target.parent().ok_or("parent")?)?;
        std::fs::write(
            target,
            STANDARD.decode(variants[digest].as_str().ok_or("bytes")?)?,
        )?;
    }
    Ok(())
}

fn answer(world: &World, args: &Value) -> Fallible<Value> {
    let mut fact = swift()?["fact"].clone();
    fact["root"] = json!(world.root);
    fact["digest"] = args["digest"].clone();
    match world.mode {
        Mode::Ready => {}
        Mode::Waiting => {
            if args["companions"].as_array().is_none_or(Vec::is_empty) {
                fact["result"]["awaitingLive"] = json!(true);
            }
        }
        Mode::Failed => {
            fact["status"] = json!("failed");
            fact.as_object_mut().ok_or("fact")?.remove("result");
        }
        Mode::Unsupported => {
            fact["status"] = json!("unsupported");
            fact.as_object_mut().ok_or("fact")?.remove("result");
        }
        Mode::Broken => {}
    }
    Ok(fact)
}

fn serve(mut stream: UnixStream, world: Shared) {
    std::thread::spawn(move || {
        while let Ok((_, frame)) = receive(&mut stream) {
            if frame["t"] == "Hello" {
                let _ = send(
                    &mut stream,
                    &json!({"t":"Welcome","protocol":"native/1","version":1}),
                );
                continue;
            }
            let op = frame["op"].as_str().unwrap_or_default().to_owned();
            let Ok(mut guard) = world.lock() else { return };
            guard.requests.push(frame.clone());
            if op != "messages.media" || guard.mode == Mode::Broken {
                let _ = send(
                    &mut stream,
                    &json!({"t":"Error","id":frame["id"],"code":"unavailable"}),
                );
                continue;
            }
            let Ok(item) = answer(&guard, &frame["args"]) else {
                return;
            };
            drop(guard);
            let _ = send(
                &mut stream,
                &json!({"t":"FactPage","id":frame["id"],"op":op,"snapshot":"snap","items":[item],"cursor":null,"complete":true,"scope":"messages.media","generation":"native/1"}),
            );
        }
    });
}

struct Wire {
    gateway: UnixStream,
    world: Shared,
    home: PathBuf,
}

fn scratch(name: &str) -> Fallible<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "ab-messages-media-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&root)?;
    Ok(root.canonicalize()?)
}

fn fixture(home: &Path, clip: bool) -> Fallible<PathBuf> {
    let root = home.join("Library/Messages");
    let directory = root.join("Attachments/aa");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("photo.png"), PHOTO)?;
    let connection = rusqlite::Connection::open(root.join("chat.db"))?;
    connection.execute_batch(
        "CREATE TABLE message(ROWID INTEGER PRIMARY KEY,guid TEXT,date INTEGER,text TEXT,handle_id INTEGER,is_from_me INTEGER,associated_message_guid TEXT,associated_message_type INTEGER);
         CREATE TABLE chat(ROWID INTEGER PRIMARY KEY,guid TEXT,service_name TEXT,display_name TEXT);
         CREATE TABLE handle(ROWID INTEGER PRIMARY KEY,id TEXT,service TEXT);
         CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
         CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
         CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY,guid TEXT,filename TEXT,mime_type TEXT,uti TEXT,total_bytes INTEGER,transfer_state INTEGER);
         CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER);
         INSERT INTO handle VALUES(1,'+15550100','iMessage');
         INSERT INTO chat VALUES(1,'iMessage;-;+15550100','iMessage','');
         INSERT INTO chat_handle_join VALUES(1,1);
         INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me) VALUES(1,'msg-1',700000000000000001,'look',1,0);
         INSERT INTO chat_message_join VALUES(1,1);
         INSERT INTO message_attachment_join VALUES(1,1);",
    )?;
    connection.execute(
        "INSERT INTO attachment VALUES(1,'att-1',?1,'image/png','public.png',?2,5)",
        rusqlite::params![
            format!("{}/photo.png", directory.display()),
            i64::try_from(PHOTO.len())?
        ],
    )?;
    if clip {
        std::fs::write(directory.join("clip.mov"), b"not a real movie")?;
        connection.execute(
            "INSERT INTO attachment VALUES(2,'att-2',?1,'video/quicktime','com.apple.quicktime-movie',16,5)",
            [format!("{}/clip.mov", directory.display())],
        )?;
    }
    Ok(root)
}

fn wire(name: &str, mode: Mode, extra: &Value) -> Fallible<Wire> {
    let home = scratch(name)?;
    let root = fixture(&home, mode == Mode::Waiting)?;
    let media = home.join("media-root");
    std::fs::create_dir_all(&media)?;
    materialize(&media)?;
    let (native, server) = UnixStream::pair()?;
    let world: Shared = Arc::new(Mutex::new(World {
        mode,
        requests: Vec::new(),
        root: media,
    }));
    serve(server, Arc::clone(&world));
    let client = Arc::new(Client::inherited(native)?);
    let mut source = json!({"root":root,"facts":"native","media_retry_ms":150});
    for (key, value) in extra.as_object().into_iter().flatten() {
        source[key] = value.clone();
    }
    let sources =
        readers::from_config_linked(&json!({"messages":source}).to_string(), Some(&client))?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let options = routed::Options {
        poll: Duration::from_millis(20),
        ..routed::Options::default()
    };
    std::thread::spawn(move || {
        routed::relay_layered(session_gateway, session_helper, sources, &options, None)
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
    std::mem::forget(helper);
    Ok(Wire {
        gateway,
        world,
        home,
    })
}

fn commit(wire: &mut Wire, capture: &Value, size: usize) -> Fallible<()> {
    send(
        &mut wire.gateway,
        &json!({"t":"Committed","source":"messages","checkpoint":capture["checkpoint"],"credit":size}),
    )
}

fn attachment(capture: &Value) -> Option<Value> {
    capture["observations"]
        .as_array()?
        .iter()
        .find(|row| row["native_key"] == "att-1" && row["kind"] == "upsert")
        .cloned()
}

struct Seen {
    first: Value,
    last: Value,
    handles: serde_json::Map<String, Value>,
    messages: usize,
}

fn follow(wire: &mut Wire, until: impl Fn(&Value) -> bool) -> Fallible<Seen> {
    let mut first = Value::Null;
    let mut last;
    let mut handles = serde_json::Map::new();
    let mut messages = 0;
    for _ in 0..40 {
        let (size, frame) = receive(&mut wire.gateway)?;
        messages += frame["observations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["native_key"] == "msg-1" && row["kind"] == "upsert")
            .count();
        if let Some(found) = frame["blob_handles"].as_object() {
            handles.extend(found.clone());
        }
        let row = attachment(&frame);
        commit(wire, &frame, size)?;
        if let Some(row) = row {
            if first.is_null() {
                first = row.clone();
            }
            last = row.clone();
            if until(&row) {
                return Ok(Seen {
                    first,
                    last,
                    handles,
                    messages,
                });
            }
        }
    }
    Err("the attachment never reached the expected state".into())
}

fn blob(wire: &mut Wire, handle: &str) -> Fallible<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        send(
            &mut wire.gateway,
            &json!({"t":"BlobRead","handle":handle,"offset":bytes.len(),"length":131_072}),
        )?;
        let chunk = loop {
            let (size, frame) = receive(&mut wire.gateway)?;
            if frame["t"] == "BlobChunk" {
                break frame;
            }
            if frame["t"] == "BlobUnavailable" {
                return Err(format!("{handle} unavailable").into());
            }
            if frame["t"] == "Capture" {
                commit(wire, &frame, size)?;
            }
        };
        bytes.extend(STANDARD.decode(chunk["bytes"].as_str().ok_or("bytes")?)?);
        if chunk["eof"] == true {
            return Ok(bytes);
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_new_image_attachment_gets_the_record_swift_gives_it() -> Fallible<()> {
    let record = swift()?;
    let mut wire = wire("ready", Mode::Ready, &json!({}))?;
    let seen = follow(&mut wire, |row| !row["data"]["media"].is_null())?;

    assert!(seen.first["data"].get("media").is_none());
    assert!(seen.first["data"].get("derivatives").is_none());
    let data = &seen.last["data"];
    assert_eq!(data["media"], record["media"]);
    assert_eq!(data["derivatives"], record["derivatives"]);
    assert_eq!(
        serde_json::to_string(&data["media"])?,
        serde_json::to_string(&record["media"])?
    );
    assert_eq!(
        serde_json::to_string(&data["derivatives"])?,
        serde_json::to_string(&record["derivatives"])?
    );
    assert_eq!(data["blob"]["state"], "present");
    assert_eq!(data["blob"]["digest"], record["source_digest"]);
    assert_eq!(
        digest(PHOTO),
        record["source_digest"].as_str().ok_or("digest")?
    );
    assert!(seen.messages >= 1);

    let requests = wire.world.lock().map_err(|_| "poisoned")?.requests.clone();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let args = &requests[0]["args"];
    assert_eq!(args["digest"], record["source_digest"]);
    assert_eq!(args["uti"], "public.png");
    assert_eq!(args["mime"], "image/png");
    assert_eq!(args["sticker"], false);
    assert_eq!(args["companions"], json!([]));
    assert!(
        args["file"]
            .as_str()
            .is_some_and(|file| file.ends_with("Attachments/aa/photo.png"))
    );

    let variants = record["variants"].as_object().ok_or("variants")?;
    assert_eq!(variants.len(), 2);
    for (variant, bytes) in variants {
        let handle = format!("media:{variant}");
        assert_eq!(seen.handles.get(variant), Some(&json!(handle)));
        let served = blob(&mut wire, &handle)?;
        assert_eq!(served, STANDARD.decode(bytes.as_str().ok_or("bytes")?)?);
        assert_eq!(&digest(&served), variant);
    }
    std::fs::remove_dir_all(&wire.home)?;
    Ok(())
}

#[test]
fn a_failed_derivation_keeps_the_record_and_is_retried() -> Fallible<()> {
    let record = swift()?;
    let mut wire = wire("failed", Mode::Failed, &json!({}))?;
    let seen = follow(&mut wire, |row| !row["data"]["derivatives"].is_null())?;
    let data = &seen.last["data"];
    assert_eq!(data["media"], json!({}));
    assert_eq!(data["derivatives"], json!({}));
    assert_eq!(data["blob"]["state"], "present");
    assert_eq!(data["blob"]["digest"], record["source_digest"]);
    assert_eq!(data["filename"], "photo.png");
    assert!(seen.messages >= 1);

    wire.world.lock().map_err(|_| "poisoned")?.mode = Mode::Ready;
    let healed = follow(&mut wire, |row| {
        row["data"]["media"].get("thumbhash").is_some()
    })?;
    assert_eq!(healed.last["data"]["media"], record["media"]);
    assert_eq!(healed.last["data"]["derivatives"], record["derivatives"]);
    assert!(wire.world.lock().map_err(|_| "poisoned")?.requests.len() >= 2);
    std::fs::remove_dir_all(&wire.home)?;
    Ok(())
}

#[test]
fn an_unreachable_helper_never_blocks_the_attachment() -> Fallible<()> {
    let mut wire = wire("broken", Mode::Broken, &json!({}))?;
    let seen = follow(&mut wire, |row| !row["data"]["derivatives"].is_null())?;
    assert_eq!(seen.last["data"]["media"], json!({}));
    assert_eq!(seen.last["data"]["derivatives"], json!({}));
    assert_eq!(seen.last["data"]["blob"]["state"], "present");
    assert!(seen.messages >= 1);
    std::fs::remove_dir_all(&wire.home)?;
    Ok(())
}

#[test]
fn an_unsupported_attachment_carries_no_media_fields() -> Fallible<()> {
    let mut wire = wire("unsupported", Mode::Unsupported, &json!({}))?;
    let seen = follow(&mut wire, |row| row["data"]["blob"]["state"] == "present")?;
    std::thread::sleep(Duration::from_millis(600));
    let data = &seen.last["data"];
    assert!(data.get("media").is_none());
    assert!(data.get("derivatives").is_none());
    assert_eq!(wire.world.lock().map_err(|_| "poisoned")?.requests.len(), 1);
    std::fs::remove_dir_all(&wire.home)?;
    Ok(())
}

#[test]
fn a_live_photo_waits_for_its_companion_and_is_asked_again_with_it() -> Fallible<()> {
    let record = swift()?;
    let mut wire = wire("live", Mode::Waiting, &json!({}))?;
    let seen = follow(&mut wire, |row| {
        row["native_key"] == "att-1" && row["data"]["media"].get("thumbhash").is_some()
    })?;
    assert_eq!(seen.last["data"]["media"], record["media"]);
    for _ in 0..200 {
        let world = wire.world.lock().map_err(|_| "poisoned")?;
        let photo = world
            .requests
            .iter()
            .filter(|request| request["args"]["digest"] == record["source_digest"])
            .count();
        drop(world);
        if photo >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let again = follow(&mut wire, |row| {
        row["native_key"] == "att-1" && row["data"]["media"].get("thumbhash").is_some()
    })?;
    assert_eq!(again.last["data"]["derivatives"], record["derivatives"]);
    let requests = wire.world.lock().map_err(|_| "poisoned")?.requests.clone();
    let asked: Vec<&Value> = requests
        .iter()
        .map(|request| &request["args"])
        .filter(|args| args["digest"] == record["source_digest"])
        .collect();
    assert!(asked.len() >= 2, "{asked:?}");
    assert_eq!(asked[0]["companions"], json!([]));
    let companions = asked[1]["companions"].as_array().ok_or("companions")?;
    assert!(
        companions
            .iter()
            .any(|path| path.as_str().is_some_and(|path| path.ends_with("clip.mov"))),
        "{companions:?}"
    );
    std::fs::remove_dir_all(&wire.home)?;
    Ok(())
}
