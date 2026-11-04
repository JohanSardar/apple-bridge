#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]

use ab_session::{
    readers::CaptureReader,
    routed::Options,
    session::{self, Attached, SystemClock},
};
use ab_wire::{crypto::ipc_proof, messages::IpcFrame};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
pub const SECRET: [u8; 32] = [7; 32];

pub fn send(socket: &mut UnixStream, value: &Value) -> Fallible<()> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

pub fn receive(socket: &mut UnixStream) -> Fallible<(usize, Value)> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header))?;
    let mut bytes = vec![0; size];
    socket.read_exact(&mut bytes)?;
    Ok((size + 4, serde_json::from_slice(&bytes)?))
}

pub fn expect_silence(socket: &mut UnixStream) -> Fallible<()> {
    socket.set_read_timeout(Some(Duration::from_millis(400)))?;
    let mut byte = [0; 1];
    let result = socket.read(&mut byte);
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    assert!(
        matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut),
        "expected silence, got {result:?}"
    );
    Ok(())
}

pub fn temporary(name: &str) -> Fallible<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ab-{name}-{}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn authenticate(gateway: &mut UnixStream) -> Fallible<()> {
    let nonce = "a".repeat(64);
    let frame = IpcFrame {
        t: "Challenge".into(),
        nonce: nonce.clone(),
        ..IpcFrame::default()
    };
    gateway.write_all(&frame.encode().map_err(|_| "encode")?)?;
    let (_, answer) = receive(gateway)?;
    let mine = answer["nonce"].as_str().ok_or("nonce")?;
    let proof = |role: &str| ipc_proof(&SECRET, role, &nonce, mine).map(|p| hex(&p));
    let proof_helper = proof("helper").map_err(|_| "proof")?;
    assert_eq!(answer["proof"], proof_helper.as_str());
    let frame = IpcFrame {
        t: "Authenticated".into(),
        proof: proof("gateway").map_err(|_| "proof")?,
        ..IpcFrame::default()
    };
    gateway.write_all(&frame.encode().map_err(|_| "encode")?)?;
    Ok(())
}

pub struct Wire {
    pub gateway: UnixStream,
    pub link: UnixStream,
    pub worker: JoinHandle<Result<(), ab_session::Error>>,
    pub home: PathBuf,
}

impl Wire {
    pub fn finish(self) -> Result<(), ab_session::Error> {
        drop(self.gateway);
        drop(self.link);
        let result = self.worker.join().expect("session thread");
        let _ = std::fs::remove_dir_all(self.home);
        result
    }

    pub fn commit(&mut self, source: &str, capture: &Value, size: usize) -> Fallible<()> {
        send(
            &mut self.gateway,
            &json!({"t":"Committed","source":source,"checkpoint":capture["checkpoint"],"credit":size}),
        )
    }

    pub fn scan(&mut self, id: &str, source: &str, extra: &Value) -> Fallible<()> {
        let mut frame =
            json!({"t":"Scan","id":id,"source":source,"generation":"swift-g","observations":[]});
        if let (Some(frame), Some(extra)) = (frame.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                frame.insert(key.clone(), value.clone());
            }
        }
        send(&mut self.link, &frame)
    }
}

pub fn wire(
    readers: BTreeMap<String, Box<dyn CaptureReader>>,
    options: Options,
    checkpoints: impl Into<Value>,
) -> Fallible<Wire> {
    wire_with(readers, options, checkpoints, &session::Options::default())
}

pub fn wire_with(
    readers: BTreeMap<String, Box<dyn CaptureReader>>,
    options: Options,
    checkpoints: impl Into<Value>,
    configured: &session::Options,
) -> Fallible<Wire> {
    let home = temporary("session-capture-home")?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut link, session_link) = UnixStream::pair()?;
    for socket in [&gateway, &link] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let attached = Attached { readers, options };
    let configured = configured.clone();
    let session_home = home.clone();
    let worker = std::thread::spawn(move || {
        session::serve_attached(
            session_gateway,
            session_link,
            &SECRET,
            &session_home,
            Arc::new(SystemClock::start()),
            &configured,
            Some(attached),
        )
    });
    authenticate(&mut gateway)?;
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":7,"byte_credit":4_194_304,"checkpoints":checkpoints.into()}),
    )?;
    let (_, hello) = receive(&mut link)?;
    assert_eq!(hello["t"], "Hello");
    send(
        &mut link,
        &json!({"t":"Welcome","protocol":"native/1","version":1}),
    )?;
    let (_, opened) = receive(&mut link)?;
    assert_eq!(opened["t"], "Session");
    send(
        &mut link,
        &json!({"t":"Ready","coverage":{},"capabilities":{},"helper_build":"t","os_build":"o"}),
    )?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
    Ok(Wire {
        gateway,
        link,
        worker,
        home,
    })
}

pub fn polling(state: Option<&Path>) -> Options {
    Options {
        poll: Duration::from_millis(20),
        watch: false,
        state: state.map(Path::to_path_buf),
        ..Options::default()
    }
}

pub fn wait_for(path: &Path) -> Fallible<()> {
    for _ in 0..200 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err("timed out waiting for file".into())
}
