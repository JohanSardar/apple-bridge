#![cfg(feature = "rust-capture")]

use ab_session::{Error, readers, routed};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct Probe {
    root: PathBuf,
    scans: Arc<AtomicUsize>,
    seen: Arc<Mutex<String>>,
}

impl readers::CaptureReader for Probe {
    fn source(&self) -> &'static str {
        "safari"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn next(&mut self, _context: &readers::ScanContext) -> Result<Option<Value>, Error> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let text = std::fs::read_to_string(self.root.join("state")).unwrap_or_default();
        if let Ok(mut seen) = self.seen.lock() {
            *seen = text;
        }
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }
}

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

fn scratch() -> Fallible<PathBuf> {
    let directory = std::env::temp_dir().join(format!(
        "ab-pacing-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

#[test]
fn constant_churn_is_paced_and_the_last_change_is_still_captured() -> Fallible<()> {
    let directory = scratch()?;
    std::fs::write(directory.join("state"), "start")?;
    let scans = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(String::new()));
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert(
        "safari".into(),
        Box::new(Probe {
            root: directory.clone(),
            scans: scans.clone(),
            seen: seen.clone(),
        }),
    );
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let options = routed::Options {
        poll: Duration::from_secs(60),
        watch: true,
        state: None,
        ..routed::Options::default()
    };
    let worker = std::thread::spawn(move || {
        routed::relay_layered(session_gateway, session_helper, sources, &options, None)
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?["t"], "Ready");
    std::thread::sleep(Duration::from_millis(700));
    let before = scans.load(Ordering::SeqCst);

    let stop = Arc::new(AtomicBool::new(false));
    let churn = {
        let directory = directory.clone();
        let stop = stop.clone();
        std::thread::spawn(move || -> std::io::Result<()> {
            let mut tick = 0_u64;
            while !stop.load(Ordering::SeqCst) {
                tick += 1;
                std::fs::write(directory.join("state"), format!("tick-{tick}"))?;
                std::fs::write(directory.join("swap.tmp"), "x")?;
                std::fs::rename(directory.join("swap.tmp"), directory.join("swap"))?;
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        })
    };
    std::thread::sleep(Duration::from_millis(3200));
    stop.store(true, Ordering::SeqCst);
    churn.join().map_err(|_| "churn thread panicked")??;
    let during = scans.load(Ordering::SeqCst) - before;
    assert!(
        during <= 6,
        "{during} scans in a 3.2 s churn window with a 60 s poll"
    );

    std::fs::write(directory.join("state"), "final")?;
    let settled = Instant::now();
    loop {
        if seen.lock().map_err(|_| "poisoned")?.as_str() == "final" {
            break;
        }
        assert!(
            settled.elapsed() < Duration::from_secs(4),
            "the trailing change was never scanned"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn usage_watches_only_the_streams_it_reads() -> Fallible<()> {
    let directory = scratch()?;
    let sources = readers::from_config(&json!({"usage":{"root":directory}}).to_string())?;
    let roots = sources.get("usage").ok_or("usage")?.watch_paths();
    assert_eq!(
        roots,
        [
            directory.join("sync"),
            directory.join("streams/restricted/App.InFocus")
        ]
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
