use crate::{
    Error, Result, hello, mux,
    store::{Nonces, Store, month},
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub const ACCEPT_RETRY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug)]
pub struct Capacity {
    pub unrouted: usize,
    pub tenant: usize,
    pub control: usize,
    pub app: usize,
    pub per_source: usize,
}
impl Default for Capacity {
    fn default() -> Self {
        Self {
            unrouted: 256,
            tenant: 256,
            control: 128,
            app: 64,
            per_source: 16,
        }
    }
}

#[must_use]
pub fn source_key(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        IpAddr::V6(address) => {
            let mut segments = address.segments();
            segments[4..].fill(0);
            IpAddr::from(segments)
        }
        address @ IpAddr::V4(_) => address,
    }
}

#[derive(Debug)]
struct SourceSlots {
    limit: usize,
    held: Arc<std::sync::Mutex<HashMap<IpAddr, usize>>>,
}
#[derive(Debug)]
struct SourceSlot {
    source: IpAddr,
    held: Arc<std::sync::Mutex<HashMap<IpAddr, usize>>>,
}
impl SourceSlots {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            held: Arc::default(),
        }
    }
    fn acquire(&self, address: IpAddr) -> Option<SourceSlot> {
        let source = source_key(address);
        let mut held = self.held.lock().ok()?;
        let count = held.entry(source).or_insert(0);
        if *count >= self.limit {
            return None;
        }
        *count += 1;
        drop(held);
        Some(SourceSlot {
            source,
            held: self.held.clone(),
        })
    }
}
impl Drop for SourceSlot {
    fn drop(&mut self) {
        if let Ok(mut held) = self.held.lock()
            && let Some(count) = held.get_mut(&self.source)
        {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.source);
            }
        }
    }
}

#[derive(Debug)]
struct Pools {
    unrouted: Arc<Semaphore>,
    tenant: Arc<Semaphore>,
    control: Arc<Semaphore>,
    app: Arc<Semaphore>,
    anonymous_sources: SourceSlots,
    tenant_sources: SourceSlots,
}
impl Pools {
    fn new(capacity: Capacity) -> Self {
        Self {
            unrouted: Arc::new(Semaphore::new(capacity.unrouted)),
            tenant: Arc::new(Semaphore::new(capacity.tenant)),
            control: Arc::new(Semaphore::new(capacity.control)),
            app: Arc::new(Semaphore::new(capacity.app)),
            anonymous_sources: SourceSlots::new(capacity.per_source),
            tenant_sources: SourceSlots::new((capacity.tenant / 2).max(1)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionDeadlines {
    pub first_request: Duration,
    pub request: Duration,
    pub idle: Duration,
}
impl Default for ConnectionDeadlines {
    fn default() -> Self {
        Self {
            first_request: Duration::from_secs(10),
            request: Duration::from_secs(60),
            idle: Duration::from_secs(30),
        }
    }
}
#[derive(Debug)]
pub struct ConnectionClock {
    deadlines: ConnectionDeadlines,
    deadline: watch::Sender<Instant>,
    in_flight: std::sync::atomic::AtomicUsize,
}
impl ConnectionClock {
    #[must_use]
    pub fn start(deadlines: ConnectionDeadlines) -> Arc<Self> {
        Arc::new(Self {
            deadlines,
            deadline: watch::Sender::new(Instant::now() + deadlines.first_request),
            in_flight: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    pub fn request_started(&self) {
        self.in_flight
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.deadline
            .send_replace(Instant::now() + self.deadlines.request);
    }
    pub fn request_finished(&self) {
        let remaining = self
            .in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
            - 1;
        if remaining == 0 {
            self.deadline
                .send_replace(Instant::now() + self.deadlines.idle);
        }
    }
    pub async fn expired(&self) {
        let mut changed = self.deadline.subscribe();
        loop {
            let deadline = *changed.borrow_and_update();
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => return,
                result = changed.changed() => { if result.is_err() { return } }
            }
        }
    }
}

#[derive(Debug)]
pub struct Session {
    pub token: String,
    pub cancel: CancellationToken,
    pub open: mpsc::Sender<mux::Open>,
    active: Arc<Semaphore>,
    capacity: u32,
    next: Mutex<Instant>,
}
#[derive(Debug)]
pub struct ControlConnection {
    pub socket: TcpStream,
    pub peer: SocketAddr,
    pub prefix: Vec<u8>,
    pub completed: Option<oneshot::Sender<()>>,
}
pub type ControlConnections = mpsc::Sender<ControlConnection>;

#[derive(Debug)]
pub struct Server {
    pub store: Arc<Mutex<Store>>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub idle: Duration,
    pub control: Option<(String, ControlConnections)>,
    pub app: Option<(String, ControlConnections)>,
    pub capacity: Capacity,
    pub nonces: Arc<Nonces>,
}
impl Server {
    pub fn new(store: Store, idle: Duration) -> Self {
        Self {
            nonces: store.nonces.clone(),
            capacity: Capacity::default(),
            store: Arc::new(Mutex::new(store)),
            sessions: Mutex::new(HashMap::new()),
            idle,
            control: None,
            app: None,
        }
    }
    pub async fn attach(
        &self,
        host: &str,
        token: &str,
        open: mpsc::Sender<mux::Open>,
        cancel: CancellationToken,
    ) -> Result<Arc<Session>> {
        let mut sessions = self.sessions.lock().await;
        let store = self.store.lock().await;
        if !store.authenticated(host, token) {
            return Err(Error::Denied);
        }
        let d = store.devices.get(host).ok_or(Error::Denied)?;
        let capacity = u32::try_from(d.limits.connections.min(256)).map_err(|_| Error::Denied)?;
        let session = Arc::new(Session {
            token: token.to_owned(),
            cancel,
            open,
            active: Arc::new(Semaphore::new(
                usize::try_from(capacity).map_err(|_| Error::Denied)?,
            )),
            capacity,
            next: Mutex::new(Instant::now()),
        });
        if let Some(old) = sessions.insert(host.to_owned(), session.clone()) {
            old.cancel.cancel();
        }
        Ok(session)
    }
    pub async fn disconnect(&self, host: &str) {
        let session = self.sessions.lock().await.remove(host);
        if let Some(session) = session {
            session.cancel.cancel();
            let _ = session.active.acquire_many(session.capacity).await;
        }
    }
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        cancel: CancellationToken,
    ) -> Result<()> {
        let pools = Arc::new(Pools::new(self.capacity));
        let mut workers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                Some(_) = workers.join_next(), if !workers.is_empty() => {},
                accepted = listener.accept() => match accepted {
                    Ok((socket,peer)) => {
                        let Some(source) = pools.anonymous_sources.acquire(peer.ip()) else { continue };
                        let Ok(unrouted) = pools.unrouted.clone().try_acquire_owned() else { continue };
                        let server = self.clone(); let stop = cancel.clone(); let pools = pools.clone();
                        workers.spawn(async move { tokio::select! { () = stop.cancelled() => {}, _ = server.forward(socket,peer,&pools,unrouted,source) => {} } });
                    }
                    Err(e) => {
                        eprintln!("relay accept failed: {e}");
                        tokio::time::sleep(ACCEPT_RETRY).await;
                    }
                }
            }
        }
        cancel.cancel();
        workers.abort_all();
        while workers.join_next().await.is_some() {}
        for session in self.sessions.lock().await.drain().map(|(_, s)| s) {
            session.cancel.cancel();
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)]
    async fn forward(
        &self,
        mut socket: TcpStream,
        peer: SocketAddr,
        pools: &Pools,
        unrouted: OwnedSemaphorePermit,
        source: SourceSlot,
    ) -> Result<()> {
        let (host, hello) =
            tokio::time::timeout(Duration::from_secs(5), hello::client_hello(&mut socket))
                .await
                .map_err(|_| Error::Denied)??;
        let terminating = [(&self.control, &pools.control), (&self.app, &pools.app)]
            .into_iter()
            .find_map(|(host_channel, pool)| {
                host_channel
                    .as_ref()
                    .filter(|(name, _)| *name == host)
                    .map(|(_, incoming)| (incoming, pool))
            });
        if let Some((incoming, pool)) = terminating {
            let _slot = pool
                .clone()
                .try_acquire_owned()
                .map_err(|_| Error::Denied)?;
            drop(unrouted);
            let _source = source;
            let (done, wait) = oneshot::channel();
            incoming
                .send(ControlConnection {
                    socket,
                    peer,
                    prefix: hello,
                    completed: Some(done),
                })
                .await
                .map_err(|_| Error::Denied)?;
            let _ = wait.await;
            return Ok(());
        }
        let _source = pools
            .tenant_sources
            .acquire(peer.ip())
            .ok_or(Error::Denied)?;
        drop(source);
        let _tenant = pools
            .tenant
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Denied)?;
        drop(unrouted);
        let host = self.store.lock().await.canonical(&host);
        let session = self
            .sessions
            .lock()
            .await
            .get(&host)
            .cloned()
            .ok_or(Error::Denied)?;
        {
            let store = self.store.lock().await;
            let d = store.devices.get(&host).ok_or(Error::Denied)?;
            if !store.authenticated(&host, &session.token)
                || (d.bytes >= d.limits.monthly_bytes && d.month == month())
            {
                return Err(Error::Denied);
            }
        }
        let _slot = session
            .active
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Denied)?;
        let (tx, rx) = oneshot::channel();
        let transfer = async {
            session
                .open
                .send(mux::Open {
                    source: peer.ip().to_canonical(),
                    reply: tx,
                })
                .await
                .map_err(|_| Error::Denied)?;
            let mut upstream = tokio::time::timeout(Duration::from_secs(10), rx)
                .await
                .map_err(|_| Error::Denied)?
                .map_err(|_| Error::Denied)?;
            let (activity, mut changed) = watch::channel(Instant::now());
            let transfer = async {
                self.write(&session, &host, &mut upstream, &hello, &activity)
                    .await?;
                let (mut pr, mut pw) = socket.split();
                let (mut ur, mut uw) = tokio::io::split(upstream);
                tokio::select! {
                    result = self.copy(&session,&host,&mut uw,&mut pr,&activity) => result,
                    result = self.copy(&session,&host,&mut pw,&mut ur,&activity) => result,
                }
            };
            let watchdog = async {
                loop {
                    let deadline = *changed.borrow_and_update() + self.idle;
                    tokio::select! {
                        () = tokio::time::sleep_until(deadline) => break,
                        result = changed.changed() => { if result.is_err() { break } },
                    }
                }
            };
            tokio::select! { result = transfer => result, () = watchdog => Err(Error::Denied) }
        };
        tokio::select! { () = session.cancel.cancelled() => Err(Error::Denied), result = transfer => result }
    }
    async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
        &self,
        session: &Session,
        host: &str,
        dst: &mut W,
        src: &mut R,
        activity: &watch::Sender<Instant>,
    ) -> Result<()> {
        let mut buf = [0; 4096];
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            activity.send_replace(Instant::now());
            self.write(session, host, dst, &buf[..n], activity).await?;
        }
    }
    async fn write<W: AsyncWrite + Unpin>(
        &self,
        session: &Session,
        host: &str,
        dst: &mut W,
        mut data: &[u8],
        activity: &watch::Sender<Instant>,
    ) -> Result<()> {
        while !data.is_empty() {
            let (n, rate) = {
                let mut store = self.store.lock().await;
                let rate = store
                    .devices
                    .get(host)
                    .ok_or(Error::Denied)?
                    .limits
                    .bytes_per_second;
                let n = store.charge(
                    host,
                    data.len()
                        .min(4096)
                        .min(usize::try_from(rate).unwrap_or(usize::MAX)),
                )?;
                (n, rate)
            };
            let deadline = {
                let mut next = session.next.lock().await;
                *next = (*next).max(Instant::now())
                    + Duration::from_nanos(
                        u64::try_from(
                            u128::try_from(n).map_err(|_| Error::Denied)? * 1_000_000_000
                                / u128::try_from(rate).map_err(|_| Error::Denied)?,
                        )
                        .map_err(|_| Error::Denied)?,
                    );
                *next
            };
            tokio::time::sleep_until(deadline).await;
            dst.write_all(&data[..n]).await?;
            activity.send_replace(Instant::now());
            data = &data[n..];
        }
        Ok(())
    }
}
