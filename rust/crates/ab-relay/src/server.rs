use crate::{
    Error, Result, hello, mux,
    store::{Store, month},
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore, mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

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
}
impl Server {
    pub fn new(store: Store, idle: Duration) -> Self {
        Self {
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
        let slots = Arc::new(Semaphore::new(256));
        let mut workers = tokio::task::JoinSet::new();
        let result = loop {
            tokio::select! {
                () = cancel.cancelled() => break Ok(()),
                Some(_) = workers.join_next(), if !workers.is_empty() => {},
                accepted = listener.accept() => match accepted {
                    Ok((socket,_)) => {
                        let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                        let server = self.clone(); let stop = cancel.clone();
                        workers.spawn(async move { let _slot = slot; tokio::select! { () = stop.cancelled() => {}, _ = server.forward(socket) => {} } });
                    }
                    Err(e) => break Err(e.into()),
                }
            }
        };
        cancel.cancel();
        workers.abort_all();
        while workers.join_next().await.is_some() {}
        for session in self.sessions.lock().await.drain().map(|(_, s)| s) {
            session.cancel.cancel();
        }
        result
    }
    async fn forward(&self, mut socket: TcpStream) -> Result<()> {
        let (host, hello) =
            tokio::time::timeout(Duration::from_secs(5), hello::client_hello(&mut socket))
                .await
                .map_err(|_| Error::Denied)??;
        let terminating = [&self.control, &self.app]
            .into_iter()
            .flatten()
            .find(|(name, _)| *name == host);
        if let Some((_, incoming)) = terminating {
            let (done, wait) = oneshot::channel();
            incoming
                .send(ControlConnection {
                    socket,
                    prefix: hello,
                    completed: Some(done),
                })
                .await
                .map_err(|_| Error::Denied)?;
            let _ = wait.await;
            return Ok(());
        }
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
                .send(mux::Open { reply: tx })
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
