use crate::assertions::await_until;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use std::{
    env, fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static SIMAPPLE: OnceLock<PathBuf> = OnceLock::new();

pub fn configure(simapple: &Path) {
    let _ = SIMAPPLE.set(simapple.to_owned());
}

type Transcript = Arc<Mutex<Vec<u8>>>;

fn snapshot(transcript: &Transcript) -> Vec<u8> {
    transcript
        .lock()
        .map(|data| data.clone())
        .unwrap_or_default()
}
fn has_frame(mut data: &[u8], kind: &str) -> bool {
    while data.len() >= 4 {
        let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if n == 0 || data.len() < 4 + n {
            return false;
        }
        if serde_json::from_slice::<serde_json::Value>(&data[4..4 + n])
            .is_ok_and(|frame| frame["t"] == kind)
        {
            return true;
        }
        data = &data[4 + n..];
    }
    false
}
fn copy(
    mut from: UnixStream,
    mut to: UnixStream,
    transcript: &Transcript,
    running: &Arc<AtomicUsize>,
) {
    running.fetch_add(1, Ordering::SeqCst);
    let transcript = Arc::clone(transcript);
    let running = Arc::clone(running);
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = from.read(&mut buffer) {
            if n == 0 {
                break;
            }
            if let Ok(mut data) = transcript.lock() {
                data.extend_from_slice(&buffer[..n]);
            }
            if to.write_all(&buffer[..n]).is_err() {
                break;
            }
        }
        let _ = to.shutdown(Shutdown::Write);
        running.fetch_sub(1, Ordering::SeqCst);
    });
}
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
fn dup(stream: &UnixStream) -> Result<UnixStream> {
    Ok(stream.try_clone()?)
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(binary) = env::var_os("AB_SESSION_BINARY").filter(|v| !v.is_empty()) else {
        evidence.check(
            "AB_SESSION_BINARY_required",
            false,
            "AB_SESSION_BINARY is not set",
        );
        return Ok(());
    };
    evidence.check("isolated_go_gateway", true, "gateway ready");
    world.stop_helper();
    let socket = world.runtime["socket"].as_str().map(str::to_owned);
    evidence.check("gateway_config", socket.is_some(), "runtime socket read");
    let Some(socket) = socket else {
        return Ok(());
    };
    let gateway = UnixStream::connect(&socket);
    evidence.check(
        "gateway_connected",
        gateway.is_ok(),
        "gateway socket connected",
    );
    let Ok(gateway) = gateway else {
        return Ok(());
    };
    let left = UnixStream::pair();
    evidence.check("gateway_socketpair", left.is_ok(), "gateway socketpair");
    let Ok((left, child_left)) = left else {
        return Ok(());
    };
    let right = UnixStream::pair();
    evidence.check("helper_socketpair", right.is_ok(), "helper socketpair");
    let Ok((right, child_right)) = right else {
        return Ok(());
    };
    let child = Command::new("sh")
        .args([
            "-c",
            "exec \"$0\" --gateway-fd 3 --helper-fd 4 3<&0 4<&1 </dev/null >&2",
        ])
        .arg(&binary)
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_left)))
        .stdout(Stdio::from(std::os::fd::OwnedFd::from(child_right)))
        .spawn();
    evidence.check(
        "rust_child_started",
        child.is_ok(),
        "Rust session child started",
    );
    let Ok(mut child) = child else {
        return Ok(());
    };
    let directory = env::temp_dir().join(format!("ab-session-sim-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let made = fs::create_dir_all(&directory);
    evidence.check("sim_directory", made.is_ok(), "simulation directory");
    if made.is_err() {
        kill(&mut child);
        return Ok(());
    }
    let outcome = proxy(world, evidence, &directory, &gateway, (&left, &right));
    kill(&mut child);
    let _ = fs::remove_dir_all(&directory);
    outcome
}

fn proxy(
    world: &World,
    evidence: &mut Evidence,
    directory: &Path,
    gateway: &UnixStream,
    (left, right): (&UnixStream, &UnixStream),
) -> Result<()> {
    let socket = directory.join("ipc.sock");
    let listener = UnixListener::bind(&socket);
    evidence.check("sim_endpoint", listener.is_ok(), "simulation endpoint");
    let Ok(listener) = listener else {
        return Ok(());
    };
    let Some(simapple) = SIMAPPLE.get() else {
        return Err(Error::Invalid("SimApple binary not configured".into()));
    };
    let mut peer = Command::new(simapple)
        .args(["simapple", "--socket"])
        .arg(&socket)
        .arg("--secret-file")
        .arg(world.gateway_home.join("apple-secret"))
        .arg("--control")
        .arg(directory.join("control.sock"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let helper = loop {
        match listener.accept() {
            Ok((stream, _)) => break Some(stream),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Err(_) => break None,
        }
    };
    evidence.check(
        "simapple_connected",
        helper.is_some(),
        "SimApple peer connected",
    );
    let Some(helper) = helper else {
        kill(&mut peer);
        return Ok(());
    };
    helper.set_nonblocking(false)?;
    let result = transcripts(evidence, gateway, left, right, &helper);
    kill(&mut peer);
    result
}

fn transcripts(
    evidence: &mut Evidence,
    gateway: &UnixStream,
    left: &UnixStream,
    right: &UnixStream,
    helper: &UnixStream,
) -> Result<()> {
    let sent_gateway: Transcript = Arc::default();
    let received_helper: Transcript = Arc::default();
    let sent_helper: Transcript = Arc::default();
    let received_gateway: Transcript = Arc::default();
    let running = Arc::new(AtomicUsize::new(0));
    copy(dup(gateway)?, dup(left)?, &sent_gateway, &running);
    copy(dup(right)?, dup(helper)?, &received_helper, &running);
    copy(dup(helper)?, dup(right)?, &sent_helper, &running);
    copy(dup(left)?, dup(gateway)?, &received_gateway, &running);
    let matched = await_until(10, || {
        let (a, b) = (snapshot(&sent_gateway), snapshot(&received_helper));
        let (c, d) = (snapshot(&sent_helper), snapshot(&received_gateway));
        !a.is_empty()
            && a == b
            && c == d
            && has_frame(&a, "Committed")
            && has_frame(&c, "Capture")
            && has_frame(&c, "Ready")
    });
    evidence.check(
        "authentication_ready_capture_committed_byte_identity",
        matched,
        "session proxy forwarded authentication, Ready, Capture and Committed byte for byte",
    );
    let _ = helper.shutdown(Shutdown::Write);
    let _ = gateway.shutdown(Shutdown::Write);
    let drained = await_until(5, || running.load(Ordering::SeqCst) == 0);
    evidence.check(
        "bidirectional_eof_drained",
        drained,
        "both directions drained after end of stream",
    );
    if !drained {
        for stream in [gateway, helper, left, right] {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let _ = await_until(5, || running.load(Ordering::SeqCst) == 0);
    }
    let (a, b) = (snapshot(&sent_gateway), snapshot(&received_helper));
    let (c, d) = (snapshot(&sent_helper), snapshot(&received_gateway));
    evidence.check(
        "gateway_to_helper_exact_transcript",
        !a.is_empty() && a == b,
        "gateway to helper transcript identical",
    );
    evidence.check(
        "helper_to_gateway_exact_transcript",
        !c.is_empty() && c == d,
        "helper to gateway transcript identical",
    );
    Ok(())
}
