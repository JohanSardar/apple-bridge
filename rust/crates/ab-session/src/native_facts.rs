use crate::{
    Error,
    frame::{read_frame, send},
};
use serde_json::{Value, json};
use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const PAGE_LIMIT: u64 = 500;
const MAX_ITEMS: usize = 100_000;
const MAX_PAGES: usize = 4_096;
const IO_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_REVALIDATE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Present,
    Absent,
    Evict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub identity: String,
    pub disposition: Disposition,
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
struct Connection {
    stream: UnixStream,
    fence: u64,
}

#[derive(Debug)]
enum Failure {
    Remote(String),
    Broken(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Broken(error)
    }
}

#[derive(Debug)]
pub struct Client {
    origin: Origin,
    connection: Mutex<Option<Connection>>,
    requests: AtomicU64,
}

impl Client {
    pub fn socket(path: PathBuf) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error::Invalid);
        }
        Ok(Self {
            origin: Origin::Socket(path),
            connection: Mutex::new(None),
            requests: AtomicU64::new(0),
        })
    }

    pub fn inherited(stream: UnixStream) -> Result<Self, Error> {
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        Ok(Self {
            origin: Origin::Inherited,
            connection: Mutex::new(Some(Connection { stream, fence: 0 })),
            requests: AtomicU64::new(0),
        })
    }

    #[must_use]
    pub fn detached() -> Self {
        Self {
            origin: Origin::Detached,
            connection: Mutex::new(None),
            requests: AtomicU64::new(0),
        }
    }

    pub fn fetch(&self, fence: u64, operation: &str) -> Result<FactSet, Error> {
        self.fetch_with(fence, operation, &json!({}))
    }

    pub fn fetch_with(&self, fence: u64, operation: &str, args: &Value) -> Result<FactSet, Error> {
        self.run(fence, |connection| {
            self.exchange(connection, fence, operation, args)
        })
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
        self.run(fence, |connection| {
            self.verdicts(connection, fence, operation, identities)
        })
    }

    fn run<T>(
        &self,
        fence: u64,
        exchange: impl FnOnce(&mut Connection) -> Result<T, Failure>,
    ) -> Result<T, Error> {
        if fence == 0 {
            return Err(Error::Invalid);
        }
        let mut guard = self.connection.lock().map_err(|_| Error::WorkerPanicked)?;
        if guard.is_none() {
            let Origin::Socket(path) = &self.origin else {
                return Err(Error::Conflict);
            };
            let stream = UnixStream::connect(path)?;
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            *guard = Some(Connection { stream, fence: 0 });
        }
        let connection = guard.as_mut().ok_or(Error::Conflict)?;
        match exchange(connection) {
            Ok(value) => Ok(value),
            Err(Failure::Remote(code)) => Err(Error::Reader(format!("native/1 {code}"))),
            Err(Failure::Broken(error)) => {
                *guard = None;
                Err(error)
            }
        }
    }

    fn verdicts(
        &self,
        connection: &mut Connection,
        fence: u64,
        operation: &str,
        identities: &[Value],
    ) -> Result<Vec<Verdict>, Failure> {
        Self::handshake(connection, fence)?;
        let id = format!("revalidate-{}", self.requests.fetch_add(1, Ordering::SeqCst) + 1);
        send(
            &mut connection.stream,
            &json!({"t":"Revalidate","id":id,"op":operation,"args":{"identities":identities}}),
        )?;
        let (_, reply) = read_frame(&mut connection.stream)?;
        if reply["id"] != json!(id) {
            return Err(Error::Invalid.into());
        }
        match reply["t"].as_str() {
            Some("Error") => {
                let code = reply["code"].as_str().unwrap_or("unavailable");
                return Err(Failure::Remote(code.to_owned()));
            }
            Some("Revalidated") if reply["op"] == json!(operation) => {}
            _ => return Err(Error::Invalid.into()),
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
            let disposition = match result["disposition"].as_str() {
                Some("present") => Disposition::Present,
                Some("absent") => Disposition::Absent,
                Some("evict") => Disposition::Evict,
                _ => return Err(Error::Invalid.into()),
            };
            verdicts.push(Verdict {
                identity: format!("{collection}/{native_key}"),
                disposition,
            });
        }
        Ok(verdicts)
    }

    fn handshake(connection: &mut Connection, fence: u64) -> Result<(), Failure> {
        if connection.fence == fence {
            return Ok(());
        }
        if connection.fence != 0 {
            return Err(Error::Conflict.into());
        }
        send(
            &mut connection.stream,
            &json!({"t":"Hello","protocol":"native/1","version":1,"owner_fence":fence}),
        )?;
        let (_, welcome) = read_frame(&mut connection.stream)?;
        if welcome != json!({"t":"Welcome","protocol":"native/1","version":1}) {
            return Err(Error::Invalid.into());
        }
        connection.fence = fence;
        Ok(())
    }

    fn exchange(
        &self,
        connection: &mut Connection,
        fence: u64,
        operation: &str,
        args: &Value,
    ) -> Result<FactSet, Failure> {
        Self::handshake(connection, fence)?;
        let mut snapshot = Value::Null;
        let mut cursor = Value::Null;
        let mut items: Vec<Value> = Vec::new();
        let mut first: Option<(Value, String)> = None;
        for _ in 0..MAX_PAGES {
            let id = format!("facts-{}", self.requests.fetch_add(1, Ordering::SeqCst) + 1);
            send(
                &mut connection.stream,
                &json!({"t":"Facts","id":id,"op":operation,"args":args,"snapshot":snapshot,"cursor":cursor,"limit":PAGE_LIMIT}),
            )?;
            let (_, page) = read_frame(&mut connection.stream)?;
            if page["id"] != json!(id) {
                return Err(Error::Invalid.into());
            }
            match page["t"].as_str() {
                Some("Error") => {
                    let code = page["code"].as_str().unwrap_or("unavailable");
                    return Err(Failure::Remote(code.to_owned()));
                }
                Some("FactPage") if page["op"] == json!(operation) => {}
                _ => return Err(Error::Invalid.into()),
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
    client: std::sync::Arc<Client>,
    operations: &'static [&'static str],
    args: Value,
    interval: Duration,
    last: Option<std::time::Instant>,
    hinted: bool,
    pulled: bool,
}

impl Pull {
    #[must_use]
    pub fn new(
        client: std::sync::Arc<Client>,
        operations: &'static [&'static str],
        interval: Duration,
    ) -> Self {
        Self {
            client,
            operations,
            args: json!({}),
            interval,
            last: None,
            hinted: false,
            pulled: false,
        }
    }

    #[must_use]
    pub fn with_args(mut self, args: Value) -> Self {
        self.args = args;
        self
    }

    #[must_use]
    pub fn client(&self) -> &std::sync::Arc<Client> {
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
        self.hinted || self.last.is_none_or(|at| at.elapsed() >= self.interval)
    }

    pub fn collect(&mut self, fence: u64) -> Result<Option<Collected>, Error> {
        self.hinted = false;
        self.last = Some(std::time::Instant::now());
        let mut generation: Option<String> = None;
        let mut rows = Vec::new();
        let mut handles = serde_json::Map::new();
        for operation in self.operations {
            let set = self.client.fetch_with(fence, operation, &self.args)?;
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
