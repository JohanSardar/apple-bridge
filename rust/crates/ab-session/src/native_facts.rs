use crate::{
    Error,
    frame::{read_frame, send},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

const PAGE_LIMIT: u64 = 500;
const MAX_ITEMS: usize = 100_000;
const MAX_PAGES: usize = 4_096;
const IO_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_REVALIDATE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Present,
    Missing,
    OutsideScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub identity: String,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactSet {
    pub items: Vec<Value>,
    pub scope: Value,
    pub generation: String,
    pub complete: bool,
}

#[derive(Debug)]
enum Origin {
    Socket(PathBuf),
    Inherited,
    Detached,
}

#[derive(Debug)]
enum Failure {
    Remote(String),
    Malformed,
    Broken(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Invalid => Self::Malformed,
            other => Self::Broken(other),
        }
    }
}

type Changes = Arc<Mutex<BTreeSet<String>>>;

#[derive(Debug)]
struct Inbox {
    waiters: Mutex<BTreeMap<String, mpsc::Sender<Value>>>,
    changed: Changes,
}

impl Inbox {
    fn route(&self, frame: Value) {
        if frame["t"] == "Changed" {
            if let (Some(source), Ok(mut changed)) = (
                frame["source"].as_str().filter(|text| !text.is_empty()),
                self.changed.lock(),
            ) {
                changed.insert(source.to_owned());
            }
            return;
        }
        let Some(id) = frame["id"].as_str() else {
            return;
        };
        if let Ok(waiters) = self.waiters.lock()
            && let Some(waiter) = waiters.get(id)
        {
            let _ = waiter.send(frame);
        }
    }

    fn close(&self) {
        if let Ok(mut waiters) = self.waiters.lock() {
            waiters.clear();
        }
    }
}

#[derive(Debug)]
struct Link {
    writer: Mutex<UnixStream>,
    fence: u64,
    inbox: Arc<Inbox>,
}

impl Drop for Link {
    fn drop(&mut self) {
        if let Ok(writer) = self.writer.lock() {
            let _ = writer.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[derive(Debug)]
enum Connection {
    Closed,
    Raw(UnixStream),
    Live(Arc<Link>),
}

#[derive(Debug)]
pub struct Client {
    origin: Origin,
    connection: Mutex<Connection>,
    changed: Changes,
    requests: AtomicU64,
}

impl Client {
    pub fn socket(path: PathBuf) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error::Invalid);
        }
        Ok(Self::new(Origin::Socket(path), Connection::Closed))
    }

    pub fn inherited(stream: UnixStream) -> Result<Self, Error> {
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        Ok(Self::new(Origin::Inherited, Connection::Raw(stream)))
    }

    #[must_use]
    pub fn detached() -> Self {
        Self::new(Origin::Detached, Connection::Closed)
    }

    fn new(origin: Origin, connection: Connection) -> Self {
        Self {
            origin,
            connection: Mutex::new(connection),
            changed: Changes::default(),
            requests: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn changed(&self, source: &str) -> bool {
        self.changed
            .lock()
            .is_ok_and(|changed| changed.contains(source))
    }

    pub fn acknowledge(&self, source: &str) {
        if let Ok(mut changed) = self.changed.lock() {
            changed.remove(source);
        }
    }

    pub fn fetch(&self, fence: u64, operation: &str) -> Result<FactSet, Error> {
        self.guarded(fence, |link| self.exchange(link, operation))
    }

    pub fn revalidate(
        &self,
        fence: u64,
        operation: &str,
        identities: &[Value],
    ) -> Result<Vec<Verdict>, Error> {
        if identities.is_empty() || identities.len() > MAX_REVALIDATE {
            return Err(Error::Invalid);
        }
        self.guarded(fence, |link| self.verdicts(link, operation, identities))
    }

    fn guarded<T>(
        &self,
        fence: u64,
        work: impl FnOnce(&Arc<Link>) -> Result<T, Failure>,
    ) -> Result<T, Error> {
        if fence == 0 {
            return Err(Error::Invalid);
        }
        let link = self.link(fence)?;
        match work(&link) {
            Ok(value) => Ok(value),
            Err(Failure::Remote(code)) => Err(Error::Reader(format!("native/1 {code}"))),
            Err(Failure::Malformed) => Err(Error::Invalid),
            Err(Failure::Broken(error)) => {
                self.drop_link(&link);
                Err(error)
            }
        }
    }

    fn drop_link(&self, link: &Arc<Link>) {
        if let Ok(mut connection) = self.connection.lock()
            && matches!(&*connection, Connection::Live(live) if Arc::ptr_eq(live, link))
        {
            *connection = Connection::Closed;
        }
    }

    fn link(&self, fence: u64) -> Result<Arc<Link>, Error> {
        let mut connection = self.connection.lock().map_err(|_| Error::WorkerPanicked)?;
        let mut stream = match std::mem::replace(&mut *connection, Connection::Closed) {
            Connection::Live(link) => {
                let same = link.fence == fence;
                *connection = Connection::Live(link.clone());
                return if same { Ok(link) } else { Err(Error::Conflict) };
            }
            Connection::Raw(stream) => stream,
            Connection::Closed => {
                let Origin::Socket(path) = &self.origin else {
                    return Err(Error::Conflict);
                };
                let stream = UnixStream::connect(path)?;
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                stream
            }
        };
        send(
            &mut stream,
            &json!({"t":"Hello","protocol":"native/1","version":1,"owner_fence":fence}),
        )?;
        let (_, welcome) = read_frame(&mut stream)?;
        if welcome != json!({"t":"Welcome","protocol":"native/1","version":1}) {
            return Err(Error::Invalid);
        }
        let mut reader = stream.try_clone()?;
        reader.set_read_timeout(None)?;
        let inbox = Arc::new(Inbox {
            waiters: Mutex::new(BTreeMap::new()),
            changed: Arc::clone(&self.changed),
        });
        let link = Arc::new(Link {
            writer: Mutex::new(stream),
            fence,
            inbox: Arc::clone(&inbox),
        });
        std::thread::spawn(move || {
            while let Ok((_, frame)) = read_frame(&mut reader) {
                inbox.route(frame);
            }
            inbox.close();
        });
        *connection = Connection::Live(Arc::clone(&link));
        Ok(link)
    }

    fn call(
        &self,
        link: &Link,
        prefix: &str,
        build: impl FnOnce(&str) -> Value,
    ) -> Result<Value, Failure> {
        let id = format!(
            "{prefix}-{}",
            self.requests.fetch_add(1, Ordering::SeqCst) + 1
        );
        let (waiter, reply) = mpsc::channel();
        link.inbox
            .waiters
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .insert(id.clone(), waiter);
        let sent = {
            let mut writer = link.writer.lock().map_err(|_| Error::WorkerPanicked)?;
            send(&mut writer, &build(&id))
        };
        let received = sent.and_then(|()| {
            reply
                .recv_timeout(IO_TIMEOUT)
                .map_err(|_| Error::Reader("native/1 stream closed".to_owned()))
        });
        if let Ok(mut waiters) = link.inbox.waiters.lock() {
            waiters.remove(&id);
        }
        let frame = received?;
        if frame["t"] == "Error" {
            let code = frame["code"].as_str().unwrap_or("unavailable");
            return Err(Failure::Remote(code.to_owned()));
        }
        Ok(frame)
    }

    fn verdicts(
        &self,
        link: &Link,
        operation: &str,
        identities: &[Value],
    ) -> Result<Vec<Verdict>, Failure> {
        let reply = self.call(
            link,
            "revalidate",
            |id| json!({"t":"Revalidate","id":id,"op":operation,"args":{"identities":identities}}),
        )?;
        if reply["t"] != "Revalidated" || reply["op"] != json!(operation) {
            return Err(Error::Invalid.into());
        }
        let results = reply["results"].as_array().ok_or(Error::Invalid)?;
        if results.len() != identities.len() {
            return Err(Error::Invalid.into());
        }
        let mut verdicts = Vec::with_capacity(results.len());
        for (result, asked) in results.iter().zip(identities) {
            let collection = result["collection"].as_str().ok_or(Error::Invalid)?;
            let native_key = result["native_key"].as_str().ok_or(Error::Invalid)?;
            if asked["collection"] != json!(collection) || asked["native_key"] != json!(native_key)
            {
                return Err(Error::Invalid.into());
            }
            let state = match result["state"].as_str() {
                Some("present") => State::Present,
                Some("missing") => State::Missing,
                Some("outside_scope") => State::OutsideScope,
                _ => return Err(Error::Invalid.into()),
            };
            verdicts.push(Verdict {
                identity: format!("{collection}/{native_key}"),
                state,
            });
        }
        Ok(verdicts)
    }

    fn exchange(&self, link: &Link, operation: &str) -> Result<FactSet, Failure> {
        let mut snapshot = Value::Null;
        let mut cursor = Value::Null;
        let mut items: Vec<Value> = Vec::new();
        let mut first: Option<(Value, String)> = None;
        for _ in 0..MAX_PAGES {
            let page = self.call(link, "facts", |id| {
                json!({"t":"Facts","id":id,"op":operation,"args":{},"snapshot":snapshot,"cursor":cursor,"limit":PAGE_LIMIT})
            })?;
            if page["t"] != "FactPage" || page["op"] != json!(operation) {
                return Err(Error::Invalid.into());
            }
            let token = page["snapshot"]
                .as_str()
                .filter(|token| !token.is_empty())
                .ok_or(Error::Invalid)?;
            let generation = page["generation"]
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or(Error::Invalid)?;
            let scope_valid = match &page["scope"] {
                Value::String(text) => !text.is_empty(),
                Value::Object(map) => !map.is_empty(),
                _ => false,
            };
            if !scope_valid {
                return Err(Error::Invalid.into());
            }
            match &first {
                None => {
                    first = Some((page["scope"].clone(), generation.to_owned()));
                    snapshot = json!(token);
                }
                Some((scope, expected)) => {
                    if *scope != page["scope"] || expected != generation || snapshot != json!(token)
                    {
                        return Err(Error::Invalid.into());
                    }
                }
            }
            let rows = page["items"].as_array().ok_or(Error::Invalid)?;
            if rows.iter().any(|row| !row.is_object())
                || items.len().saturating_add(rows.len()) > MAX_ITEMS
            {
                return Err(Error::Invalid.into());
            }
            items.extend(rows.iter().cloned());
            let complete = page["complete"].as_bool().ok_or(Error::Invalid)?;
            match &page["cursor"] {
                Value::Null => {
                    let (scope, generation) = first.ok_or(Error::Invalid)?;
                    return Ok(FactSet {
                        items,
                        scope,
                        generation,
                        complete,
                    });
                }
                Value::String(next) if !complete && next.starts_with(&format!("{token}:")) => {
                    cursor = json!(next);
                }
                _ => return Err(Error::Invalid.into()),
            }
        }
        Err(Error::Invalid.into())
    }
}

#[derive(Debug)]
pub struct Collected {
    pub generation: String,
    pub rows: Vec<Value>,
    pub handles: serde_json::Map<String, Value>,
}

#[derive(Debug)]
pub struct Pull {
    client: Arc<Client>,
    source: &'static str,
    operations: &'static [&'static str],
    interval: Duration,
    last: Option<std::time::Instant>,
    hinted: bool,
    pulled: bool,
}

impl Pull {
    #[must_use]
    pub fn new(
        client: Arc<Client>,
        source: &'static str,
        operations: &'static [&'static str],
        interval: Duration,
    ) -> Self {
        Self {
            client,
            source,
            operations,
            interval,
            last: None,
            hinted: false,
            pulled: false,
        }
    }

    #[must_use]
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    pub fn hint(&mut self) {
        self.hinted = true;
    }

    #[must_use]
    pub fn live(&self) -> bool {
        self.pulled
    }

    pub fn due(&self) -> bool {
        self.hinted
            || self.client.changed(self.source)
            || self.last.is_none_or(|at| at.elapsed() >= self.interval)
    }

    pub fn collect(&mut self, fence: u64) -> Result<Option<Collected>, Error> {
        self.hinted = false;
        self.client.acknowledge(self.source);
        self.last = Some(std::time::Instant::now());
        let mut generation: Option<String> = None;
        let mut rows = Vec::new();
        let mut handles = serde_json::Map::new();
        for operation in self.operations {
            let set = self.client.fetch(fence, operation)?;
            if !set.complete {
                return Ok(None);
            }
            match &generation {
                None => generation = Some(set.generation),
                Some(expected) if *expected == set.generation => {}
                Some(_) => return Err(Error::Invalid),
            }
            for item in set.items {
                if item["kind"] == "blob_handle" {
                    let digest = item["digest"].as_str().filter(|text| !text.is_empty());
                    let handle = item["handle"].as_str().filter(|text| !text.is_empty());
                    let (Some(digest), Some(handle)) = (digest, handle) else {
                        return Err(Error::Invalid);
                    };
                    handles.insert(digest.to_owned(), Value::String(handle.to_owned()));
                } else {
                    rows.push(item);
                }
            }
        }
        self.pulled = true;
        Ok(generation.map(|generation| Collected {
            generation,
            rows,
            handles,
        }))
    }
}
