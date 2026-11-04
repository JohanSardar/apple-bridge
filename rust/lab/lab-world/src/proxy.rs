use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct State {
    target: Mutex<String>,
    partitioned: AtomicBool,
    blackhole: AtomicBool,
    delay_ms: AtomicU64,
    delay_both: AtomicBool,
    closed: AtomicBool,
    next: AtomicU64,
    established: AtomicU64,
    upstream_closed: AtomicU64,
    connections: Mutex<HashMap<u64, TcpStream>>,
}

#[derive(Debug, Clone)]
pub struct FaultProxy {
    address: String,
    state: Arc<State>,
}

impl FaultProxy {
    pub fn start(target: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let state = Arc::new(State {
            target: Mutex::new(target.to_owned()),
            partitioned: AtomicBool::new(false),
            blackhole: AtomicBool::new(false),
            delay_ms: AtomicU64::new(0),
            delay_both: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            next: AtomicU64::new(0),
            established: AtomicU64::new(0),
            upstream_closed: AtomicU64::new(0),
            connections: Mutex::new(HashMap::new()),
        });
        let accepting = state.clone();
        thread::spawn(move || {
            for downstream in listener.incoming() {
                if accepting.closed.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(downstream) = downstream {
                    let state = accepting.clone();
                    thread::spawn(move || forward(&state, downstream));
                }
            }
        });
        Ok(Self { address, state })
    }

    pub fn retarget(&self, target: &str) {
        target.clone_into(&mut lock(&self.state.target));
    }

    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn partition(&self) {
        self.state.partitioned.store(true, Ordering::SeqCst);
        let mut connections = lock(&self.state.connections);
        for stream in connections.values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        connections.clear();
    }

    pub fn delay(&self, delay: Duration, both: bool) {
        self.state.delay_ms.store(
            u64::try_from(delay.as_millis()).unwrap_or(0),
            Ordering::SeqCst,
        );
        self.state.delay_both.store(both, Ordering::SeqCst);
    }

    pub fn blackhole(&self) {
        self.state.blackhole.store(true, Ordering::SeqCst);
    }

    pub fn heal(&self) {
        self.state.partitioned.store(false, Ordering::SeqCst);
        self.state.blackhole.store(false, Ordering::SeqCst);
    }

    #[must_use]
    pub fn established(&self) -> u64 {
        self.state.established.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn upstream_closed(&self) -> u64 {
        self.state.upstream_closed.load(Ordering::SeqCst)
    }

    pub fn close(&self) {
        self.state.closed.store(true, Ordering::SeqCst);
        self.partition();
        let _ = TcpStream::connect(&self.address);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn forward(state: &Arc<State>, downstream: TcpStream) {
    if state.partitioned.load(Ordering::SeqCst) {
        let _ = downstream.shutdown(Shutdown::Both);
        return;
    }
    let target = lock(&state.target).clone();
    let Ok(upstream) = TcpStream::connect(&target) else {
        return;
    };
    let id = state.next.fetch_add(1, Ordering::SeqCst);
    state.established.fetch_add(1, Ordering::SeqCst);
    let (Ok(down_a), Ok(down_b), Ok(up_a), Ok(up_b)) = (
        downstream.try_clone(),
        downstream.try_clone(),
        upstream.try_clone(),
        upstream.try_clone(),
    ) else {
        return;
    };
    {
        let mut connections = lock(&state.connections);
        if state.partitioned.load(Ordering::SeqCst) {
            return;
        }
        connections.insert(id, downstream);
        connections.insert(id | 1 << 63, upstream);
    }
    let forward_state = state.clone();
    let first = thread::spawn(move || pipe(&forward_state, &down_a, &up_a, true));
    pipe(state, &up_b, &down_b, false);
    let _ = first.join();
    let mut connections = lock(&state.connections);
    connections.remove(&id);
    connections.remove(&(id | 1 << 63));
}

fn pipe(state: &State, from: &TcpStream, to: &TcpStream, to_upstream: bool) {
    let (sender, receiver) = mpsc::channel::<(Vec<u8>, Instant)>();
    let Ok(sink) = to.try_clone() else {
        return;
    };
    let writer = thread::spawn(move || {
        let mut sink = sink;
        for (bytes, at) in receiver {
            let wait = at.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                thread::sleep(wait);
            }
            if sink.write_all(&bytes).is_err() {
                break;
            }
        }
        let _ = sink.shutdown(Shutdown::Both);
    });
    let mut source = from;
    let mut buffer = vec![0_u8; 32 * 1024];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => {
                if !to_upstream {
                    state.upstream_closed.fetch_add(1, Ordering::SeqCst);
                }
                break;
            }
            Ok(n) => {
                if state.blackhole.load(Ordering::SeqCst) {
                    continue;
                }
                let both = state.delay_both.load(Ordering::SeqCst);
                let delay = if to_upstream && !both {
                    0
                } else {
                    state.delay_ms.load(Ordering::SeqCst)
                };
                let at = Instant::now() + Duration::from_millis(delay);
                if sender.send((buffer[..n].to_vec(), at)).is_err() {
                    break;
                }
            }
        }
    }
    drop(sender);
    let _ = writer.join();
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}
