use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

#[derive(Debug)]
struct State {
    target: String,
    partitioned: AtomicBool,
    closed: AtomicBool,
    next: AtomicU64,
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
            target: target.to_owned(),
            partitioned: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            next: AtomicU64::new(0),
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

    pub fn heal(&self) {
        self.state.partitioned.store(false, Ordering::SeqCst);
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
    let Ok(upstream) = TcpStream::connect(&state.target) else {
        return;
    };
    let id = state.next.fetch_add(1, Ordering::SeqCst);
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
    let first = thread::spawn(move || pipe(&down_a, &up_a));
    pipe(&up_b, &down_b);
    let _ = first.join();
    let mut connections = lock(&state.connections);
    connections.remove(&id);
    connections.remove(&(id | 1 << 63));
}

fn pipe(from: &TcpStream, to: &TcpStream) {
    let mut source = from;
    let mut sink = to;
    let mut buffer = vec![0_u8; 32 * 1024];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if sink.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}
