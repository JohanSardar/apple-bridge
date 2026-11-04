#![cfg(feature = "rust-capture")]

use ab_session::{native_facts::Client, readers, routed};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

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

fn silent(socket: &mut UnixStream) -> Fallible<bool> {
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut byte = [0; 1];
    let result = socket.read(&mut byte);
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    Ok(
        matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut),
    )
}

#[derive(Default)]
struct World {
    mac: Vec<Value>,
    statuses: BTreeMap<String, String>,
    requests: Vec<Value>,
}

type Shared = Arc<Mutex<World>>;

fn serve(mut stream: UnixStream, world: Shared) {
    std::thread::spawn(move || {
        let mut snapshots: BTreeMap<String, Vec<Value>> = BTreeMap::new();
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
            let items: Vec<Value> = match op.as_str() {
                "mac.state" => guard.mac.clone(),
                "files.ubiquity" => frame["args"]["paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|path| {
                        let key = path.as_str().unwrap_or_default();
                        json!({"path":key,"status":guard.statuses.get(key).map_or("local", String::as_str)})
                    })
                    .collect(),
                _ => {
                    let _ = send(&mut stream, &json!({"t":"Error","id":frame["id"],"code":"unsupported"}));
                    continue;
                }
            };
            drop(guard);
            let token = format!("snap-{}", frame["id"].as_str().unwrap_or_default());
            snapshots.insert(token.clone(), items.clone());
            let _ = send(
                &mut stream,
                &json!({"t":"FactPage","id":frame["id"],"op":op,"snapshot":token,"items":items,"cursor":null,"complete":true,"scope":"scope","generation":"native/1"}),
            );
        }
    });
}

struct Wire {
    gateway: UnixStream,
    helper: UnixStream,
    push: UnixStream,
    world: Shared,
}

fn wire(config: &Value) -> Fallible<Wire> {
    let (native, server) = UnixStream::pair()?;
    let push = server.try_clone()?;
    let world: Shared = Arc::new(Mutex::new(World::default()));
    serve(server, Arc::clone(&world));
    let client = Arc::new(Client::inherited(native)?);
    let sources = readers::from_config_linked(&config.to_string(), Some(&client))?;
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
    Ok(Wire {
        gateway,
        helper,
        push,
        world,
    })
}

fn commit(wire: &mut Wire, source: &str, capture: &Value, size: usize) -> Fallible<()> {
    send(
        &mut wire.gateway,
        &json!({"t":"Committed","source":source,"checkpoint":capture["checkpoint"],"credit":size}),
    )
}

fn keys(capture: &Value, kind: &str) -> Vec<String> {
    let mut found: Vec<String> = capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["kind"] == kind)
        .filter_map(|o| o["native_key"].as_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

fn scratch(name: &str) -> Fallible<PathBuf> {
    let root = std::env::temp_dir().join(format!("ab-files-mac-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root.canonicalize()?)
}

fn mac_row(collection: &str, key: &str, data: &Value) -> Value {
    json!({"kind":"upsert","collection":collection,"native_key":key,"data":data})
}

#[test]
fn mac_facts_are_diffed_in_rust_and_changed_wakes_the_pull() -> Fallible<()> {
    let root = scratch("mac")?;
    let mut w = wire(&json!({"mac":{"root":root,"facts":"native","facts_poll_ms":600_000}}))?;
    w.world.lock().map_err(|_| "poisoned")?.mac = vec![
        mac_row("mac.power", "system", &json!({"charging":false})),
        mac_row("mac.app", "10", &json!({"name":"Mail"})),
        mac_row("mac.app", "11", &json!({"name":"Notes"})),
    ];
    let (size, first) = receive(&mut w.gateway)?;
    assert_eq!(first["source"], "mac");
    assert_eq!(first["generation"], "native/1");
    assert_eq!(first["live"], false);
    assert_eq!(keys(&first, "upsert"), ["10", "11", "system"]);
    let alias = first["observations"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["native_key"] == "10"))
        .map(|row| row["scoped_alias"].clone());
    assert_eq!(alias, Some(json!("mac/mac.app/10")));
    commit(&mut w, "mac", &first, size)?;
    assert!(silent(&mut w.gateway)?);

    w.world.lock().map_err(|_| "poisoned")?.mac = vec![
        mac_row("mac.power", "system", &json!({"charging":true})),
        mac_row("mac.app", "10", &json!({"name":"Mail"})),
    ];
    send(
        &mut w.push,
        &json!({"t":"Changed","source":"mac","generation":"native/1"}),
    )?;
    let (size, second) = receive(&mut w.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(second["live"], true);
    assert_eq!(keys(&second, "upsert"), ["system"]);
    assert_eq!(keys(&second, "absent"), ["11"]);
    commit(&mut w, "mac", &second, size)?;

    send(
        &mut w.push,
        &json!({"t":"Changed","source":"mac","generation":"native/1"}),
    )?;
    assert!(silent(&mut w.gateway)?);
    drop(w.helper);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn a_mac_row_outside_the_scope_is_refused() -> Fallible<()> {
    let root = scratch("macscope")?;
    let mut w = wire(&json!({"mac":{"root":root,"facts":"native","facts_poll_ms":20}}))?;
    w.world.lock().map_err(|_| "poisoned")?.mac =
        vec![mac_row("mac.bluetooth_device", "x", &json!({}))];
    assert!(silent(&mut w.gateway)?);
    drop(w.helper);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

fn paths_of(world: &Shared) -> Fallible<Vec<String>> {
    let guard = world.lock().map_err(|_| "poisoned")?;
    Ok(guard
        .requests
        .iter()
        .filter(|request| request["op"] == "files.ubiquity")
        .flat_map(|request| {
            request["args"]["paths"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|path| path.as_str().map(str::to_owned))
        .collect())
}

fn write(path: &Path, bytes: &[u8]) -> Fallible<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn rust_walks_files_and_asks_swift_only_for_ubiquity() -> Fallible<()> {
    let root = scratch("files")?;
    write(&root.join("a.txt"), b"alpha")?;
    write(&root.join("sub/b.pdf"), b"beta!!")?;
    std::os::unix::fs::symlink("/etc/passwd", root.join("link"))?;
    let facts_home = scratch("fileshome")?;
    let mut w = wire(&json!({"files":{"root":root,"facts":"native","home":facts_home}}))?;
    let b = root.join("sub/b.pdf").to_string_lossy().into_owned();
    w.world
        .lock()
        .map_err(|_| "poisoned")?
        .statuses
        .insert(b.clone(), "notDownloaded".into());
    let (size, first) = receive(&mut w.gateway)?;
    assert_eq!(first["source"], "files");
    assert_eq!(first["scope"], json!(["files.item"]));
    assert_eq!(first["completion_evidence"], "complete");
    let a = root.join("a.txt").to_string_lossy().into_owned();
    let sub = root.join("sub").to_string_lossy().into_owned();
    let mut expected = vec![a.clone(), b.clone(), sub];
    expected.sort();
    assert_eq!(keys(&first, "upsert"), expected);
    let row = |key: &str| {
        first["observations"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["native_key"] == key))
            .map_or(Value::Null, |row| row["data"].clone())
    };
    assert_eq!(row(&a)["kind"], "txt");
    assert_eq!(row(&a)["size"], 5);
    assert_eq!(row(&a)["path"], "a.txt");
    assert_eq!(row(&a)["icloud_status"], "local");
    assert_eq!(row(&a)["downloaded"], true);
    assert_eq!(row(&a)["tags"], json!([]));
    assert_eq!(row(&b)["icloud_status"], "notDownloaded");
    assert_eq!(row(&b)["downloaded"], false);
    assert_eq!(
        row(&root.join("sub").to_string_lossy())["kind"],
        "directory"
    );
    assert_eq!(row(&root.join("sub").to_string_lossy())["size"], 0);
    assert!(
        !paths_of(&w.world)?
            .iter()
            .any(|path| path.ends_with("/link"))
    );
    commit(&mut w, "files", &first, size)?;
    assert!(silent(&mut w.gateway)?);

    std::fs::remove_file(root.join("a.txt"))?;
    write(&root.join("c.md"), b"gamma")?;
    let (size, second) = receive(&mut w.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(second["live"], true);
    assert_eq!(
        keys(&second, "upsert"),
        [root.join("c.md").to_string_lossy().into_owned()]
    );
    assert_eq!(keys(&second, "absent"), [a]);
    commit(&mut w, "files", &second, size)?;
    assert!(silent(&mut w.gateway)?);
    drop(w.helper);
    std::fs::remove_dir_all(&root)?;
    std::fs::remove_dir_all(&facts_home)?;
    Ok(())
}

#[test]
fn configured_roots_come_from_the_helper_roots_file() -> Fallible<()> {
    let root = scratch("filesmain")?;
    let extra = scratch("filesextra")?;
    write(&extra.join("x.txt"), b"x")?;
    let home = scratch("filesroots")?;
    write(
        &home.join("Library/Application Support/AppleBridge/files-roots.json"),
        serde_json::to_string(&[extra.to_string_lossy()])?.as_bytes(),
    )?;
    let mut w = wire(&json!({"files":{"root":root,"facts":"native","home":home}}))?;
    let (_, capture) = receive(&mut w.gateway)?;
    assert_eq!(
        keys(&capture, "upsert"),
        [extra.join("x.txt").to_string_lossy().into_owned()]
    );
    drop(w.helper);
    for dir in [&root, &extra, &home] {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

#[test]
fn a_files_config_without_native_facts_is_refused() {
    assert!(readers::from_config(&json!({"files":{"root":"/tmp"}}).to_string()).is_err());
    assert!(readers::from_config(&json!({"mac":{"root":"/tmp"}}).to_string()).is_err());
}
