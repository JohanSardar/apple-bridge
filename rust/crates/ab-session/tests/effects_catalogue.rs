#![cfg(feature = "rust-effects")]

use ab_session::{
    Error,
    effects::{self, COMMANDS, Config, EFFECT_QUERIES, Layer, Routing},
    routed,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
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

fn temporary(name: &str) -> Fallible<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "ab-catalogue-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn notes_store(root: &Path) -> Fallible<PathBuf> {
    let path = root.join("NoteStore.sqlite");
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Synthetic');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8) VALUES(10,1,'folder','Synthetic',1);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(20,2,'note','Synthetic',10,1);")?;
    db.execute(
        "INSERT INTO ZICNOTEDATA VALUES(1,?,1)",
        [include_bytes!("../../../../lab/fixtures/notes/native-plain.gz").as_slice()],
    )?;
    Ok(path)
}

struct Harness {
    gateway: UnixStream,
    helper: UnixStream,
    native: UnixStream,
    relay: Option<std::thread::JoinHandle<Result<(), Error>>>,
    home: PathBuf,
}

impl Harness {
    fn new(config: Config) -> Fallible<Self> {
        let home = temporary("home")?;
        let (gateway, session_gateway) = UnixStream::pair()?;
        let (helper, session_helper) = UnixStream::pair()?;
        let (native, session_native) = UnixStream::pair()?;
        for socket in [&gateway, &helper, &native] {
            socket.set_read_timeout(Some(Duration::from_secs(10)))?;
        }
        let route_home = home.clone();
        let relay = std::thread::spawn(move || {
            let layer = Layer::open_with(session_native, &route_home, config)?;
            routed::relay_layered(
                session_gateway,
                session_helper,
                std::collections::BTreeMap::new(),
                &routed::Options::default(),
                Some(&layer),
            )
        });
        let mut harness = Self {
            gateway,
            helper,
            native,
            relay: Some(relay),
            home,
        };
        send(
            &mut harness.gateway,
            &json!({"t":"Open","protocol":"ipc/1","owner_fence":7}),
        )?;
        assert_eq!(receive(&mut harness.native)?["t"], "Hello");
        send(
            &mut harness.native,
            &json!({"t":"Welcome","protocol":"native/1","version":1}),
        )?;
        assert_eq!(receive(&mut harness.helper)?["t"], "Open");
        send(&mut harness.helper, &json!({"t":"Ready"}))?;
        assert_eq!(receive(&mut harness.gateway)?["t"], "Ready");
        Ok(harness)
    }

    fn prepare(&mut self, id: &str, action: &Value) -> Fallible<Value> {
        let command = json!({"t":"Prepare","command_id":id,"principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":action});
        send(&mut self.gateway, &command)?;
        assert_eq!(receive(&mut self.helper)?, command);
        send(
            &mut self.helper,
            &json!({"t":"Prepared","command_id":id,"ticket":format!("ticket-{id}"),"fingerprint":"ipc-hash","native_state":{}}),
        )?;
        assert_eq!(receive(&mut self.gateway)?["t"], "Prepared");
        Ok(command)
    }

    fn dispatch(&mut self, id: &str) -> Fallible<()> {
        send(
            &mut self.gateway,
            &json!({"t":"Dispatch","command_id":id,"ticket":format!("ticket-{id}"),"origin":{"kind":"fixture"},"owner_fence":7}),
        )
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.gateway.shutdown(Shutdown::Both);
        let _ = self.helper.shutdown(Shutdown::Both);
        let _ = self.native.shutdown(Shutdown::Both);
        if let Some(relay) = self.relay.take() {
            let _ = relay.join();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn rust_catalogue_equals_the_swift_native_catalogue() -> Fallible<()> {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../native/Sources/NativeProtocol/NativeServer.swift"
    ))?;
    let start = source
        .find("effectOperations: Set<String> = [")
        .ok_or("catalogue")?;
    let body = &source[start..];
    let body = &body[..body.find("\n    ]").ok_or("end")?];
    let swift: BTreeSet<&str> = body.split('"').skip(1).step_by(2).collect();
    let rust: BTreeSet<&str> = COMMANDS.iter().chain(EFFECT_QUERIES).copied().collect();
    assert_eq!(swift, rust);
    assert_eq!(COMMANDS.len() + EFFECT_QUERIES.len(), rust.len());
    Ok(())
}

#[test]
fn routing_names_sources_and_unknown_commands_are_refused_without_the_helper() -> Fallible<()> {
    assert!(Routing::all().routes("notes.create"));
    let only = Routing::parse("calendar, mail");
    assert!(only.routes("calendar.create_event") && only.routes("mail.send"));
    assert!(!only.routes("reminders.create") && !only.routes("mac.screen_record"));
    assert!(Routing::parse("capture").routes("mac.screen_record"));
    assert_eq!(effects::source_of("print.document"), "capture");

    let mut harness = Harness::new(Config {
        routing: Routing::only(["calendar".to_owned(), "mail".to_owned()]),
        ..Config::default()
    })?;
    send(
        &mut harness.gateway,
        &json!({"t":"Prepare","command_id":"unknown","principal":"fixture","origin":{"kind":"fixture"},"request_hash":"h","expires_at":"2099-01-01T00:00:00Z","action":{"op":"mail.forward","args":{}}}),
    )?;
    let rejected = receive(&mut harness.gateway)?;
    assert_eq!(rejected["t"], "Rejected");
    assert_eq!(rejected["error"]["code"], "unsupported");
    assert_eq!(rejected["disposition"], "rejected_before_dispatch");

    let forwarded = harness.prepare("legacy", &json!({"op":"reminders.create","args":{}}))?;
    assert_eq!(forwarded["action"]["op"], "reminders.create");
    send(
        &mut harness.gateway,
        &json!({"t":"Dispatch","command_id":"legacy","ticket":"ticket-legacy","origin":{"kind":"fixture"},"owner_fence":7}),
    )?;
    assert_eq!(receive(&mut harness.helper)?["t"], "Dispatch");
    send(
        &mut harness.gateway,
        &json!({"t":"Resolve","command_id":"legacy"}),
    )?;
    assert_eq!(receive(&mut harness.helper)?["t"], "Resolve");
    send(
        &mut harness.gateway,
        &json!({"t":"Query","id":"q","op":"maps.search","args":{}}),
    )?;
    assert_eq!(receive(&mut harness.helper)?["t"], "Query");
    Ok(())
}

#[test]
fn evidence_only_commands_keep_the_source_disposition() -> Fallible<()> {
    let mut harness = Harness::new(Config::default())?;
    harness.prepare(
        "maps",
        &json!({"op":"maps.open","args":{"lat":1.5,"lon":2.0}}),
    )?;
    harness.dispatch("maps")?;
    let effect = receive(&mut harness.native)?;
    assert_eq!(effect["args"]["action"]["args"]["lon"], 2);
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"maps","evidence":{"disposition":"locally_observed","frontmost_bundle":"com.apple.Maps"}}),
    )?;
    let outcome = receive(&mut harness.gateway)?;
    assert_eq!(outcome["disposition"], "locally_observed");
    assert_eq!(outcome["evidence"]["frontmost_bundle"], "com.apple.Maps");

    harness.prepare("message", &json!({"op":"messages.send","args":{}}))?;
    harness.dispatch("message")?;
    receive(&mut harness.native)?;
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"message","evidence":{"source":"messages","observed":false,"disposition":"failed_no_effect","reason":"staging_failed"}}),
    )?;
    let outcome = receive(&mut harness.gateway)?;
    assert_eq!(outcome["disposition"], "failed_no_effect");

    harness.prepare("say", &json!({"op":"speech.say","args":{}}))?;
    harness.dispatch("say")?;
    receive(&mut harness.native)?;
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"say","evidence":{"observed":true,"fingerprint":"ipc-hash","finished_callback":false}}),
    )?;
    assert_eq!(
        receive(&mut harness.gateway)?["disposition"],
        "indeterminate"
    );
    Ok(())
}

#[test]
fn pre_dispatch_rejection_from_the_native_side_is_durable_and_resolvable() -> Fallible<()> {
    let mut harness = Harness::new(Config::default())?;
    harness.prepare(
        "late",
        &json!({"op":"calendar.update_event","action_args":1}),
    )?;
    harness.dispatch("late")?;
    receive(&mut harness.native)?;
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"late","evidence":{"rejected":{"code":"conflict","message":"conflict"}}}),
    )?;
    let rejected = receive(&mut harness.gateway)?;
    assert_eq!(rejected["t"], "Rejected");
    assert_eq!(rejected["error"]["code"], "conflict");
    send(
        &mut harness.gateway,
        &json!({"t":"Resolve","command_id":"late"}),
    )?;
    assert_eq!(receive(&mut harness.gateway)?, rejected);

    harness.prepare("shipped", &json!({"op":"messages.send","args":{}}))?;
    harness.dispatch("shipped")?;
    receive(&mut harness.native)?;
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"shipped","evidence":{"disposition":"locally_observed","observed":false}}),
    )?;
    let first = receive(&mut harness.gateway)?;
    assert_eq!(first["disposition"], "locally_observed");
    send(
        &mut harness.gateway,
        &json!({"t":"Resolve","command_id":"shipped"}),
    )?;
    let resolve = receive(&mut harness.helper)?;
    assert_eq!(resolve["saved"], first);
    let delivered = json!({"t":"Outcome","command_id":"shipped","disposition":"delivered","evidence":{"observed":true}});
    send(&mut harness.helper, &delivered)?;
    assert_eq!(receive(&mut harness.gateway)?, delivered);
    send(
        &mut harness.gateway,
        &json!({"t":"Resolve","command_id":"shipped"}),
    )?;
    assert_eq!(
        receive(&mut harness.helper)?["saved"]["disposition"],
        "delivered"
    );
    Ok(())
}

#[test]
fn query_blobs_travel_with_the_native_result() -> Fallible<()> {
    let mut harness = Harness::new(Config::default())?;
    send(
        &mut harness.gateway,
        &json!({"t":"Query","id":"snap","op":"maps.snapshot","args":{}}),
    )?;
    receive(&mut harness.native)?;
    let blob = json!({"digest":"d","size":3,"handle":"maps-snapshot:1"});
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"snap","evidence":{"observed":true,"native_query_result":{"ok":true},"native_query_blobs":[blob]}}),
    )?;
    assert_eq!(
        receive(&mut harness.gateway)?,
        json!({"t":"QueryResult","id":"snap","result_json":{"ok":true},"blobs":[blob]})
    );
    Ok(())
}

#[test]
fn notes_pre_state_is_read_by_rust_before_the_permit_and_bound_into_it() -> Fallible<()> {
    let root = temporary("notes")?;
    let database = notes_store(&root)?;
    let mut harness = Harness::new(Config {
        notes_database: Some(database.clone()),
        ..Config::default()
    })?;
    let action = json!({"op":"notes.update","native_key":"note","note_args":{"plain_text":"x"}});
    harness.prepare("notes-1", &action)?;
    harness.dispatch("notes-1")?;
    let effect = receive(&mut harness.native)?;
    let state =
        ab_native::notes_reader::pre_state(&database, "notes.note", "note")?.ok_or("note")?;
    assert_eq!(effect["args"]["rust_pre_state"], state);
    assert_eq!(
        effect["permit"]["request_hash"],
        effects::request_hash(&effect["args"])?
    );
    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"notes-1","evidence":{"rejected":{"code":"conflict","message":"conflict"}}}),
    )?;
    assert_eq!(receive(&mut harness.gateway)?["t"], "Rejected");

    let connection = rusqlite::Connection::open(&database)?;
    connection.execute(
        "UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='Edited',Z_OPT=2 WHERE ZIDENTIFIER='note'",
        [],
    )?;
    drop(connection);
    harness.prepare("notes-2", &action)?;
    harness.dispatch("notes-2")?;
    let second = receive(&mut harness.native)?;
    assert_ne!(second["args"]["rust_pre_state"], state);

    send(
        &mut harness.native,
        &json!({"t":"EffectResult","id":"notes-2","evidence":{"rejected":{"code":"conflict","message":"conflict"}}}),
    )?;
    receive(&mut harness.gateway)?;
    harness.prepare(
        "missing",
        &json!({"op":"notes.delete","native_key":"absent"}),
    )?;
    harness.dispatch("missing")?;
    let missing = receive(&mut harness.gateway)?;
    assert_eq!(missing["t"], "Rejected");
    assert_eq!(missing["error"]["code"], "conflict");
    std::fs::remove_dir_all(root)?;
    Ok(())
}
