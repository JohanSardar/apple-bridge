use crate::{Error, readers::BlobServer};
use ab_wire::gateway::GatewayFrame;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MAIL_BODY: &str = "mail.body";
pub const SHORTCUT_ACTIONS: &str = "shortcuts.actions";
const INLINE_LIMIT: usize = 1 << 20;
const HELD_LIMIT: usize = 64 << 20;
const HOLD: Duration = Duration::from_secs(60);
const HANDLE: &str = "query-body:";

#[derive(Clone, Debug, Default)]
pub struct NativeQueries {
    pub mail_bodies: Option<Arc<MailBodies>>,
    pub shortcut_roots: Option<Vec<PathBuf>>,
}

impl NativeQueries {
    #[must_use]
    pub fn handles(&self, frame: &GatewayFrame) -> bool {
        let GatewayFrame::Query(query) = frame else {
            return false;
        };
        self.handles_operation(&query.op)
    }

    #[must_use]
    pub fn handles_operation(&self, operation: &str) -> bool {
        match operation {
            MAIL_BODY => self.mail_bodies.is_some(),
            SHORTCUT_ACTIONS => self.shortcut_roots.is_some(),
            _ => false,
        }
    }

    #[must_use]
    pub fn answer(&self, frame: &Value) -> Value {
        match (
            frame["op"].as_str(),
            &self.mail_bodies,
            &self.shortcut_roots,
        ) {
            (Some(MAIL_BODY), Some(bodies), _) => bodies.answer(frame),
            (Some(SHORTCUT_ACTIONS), _, Some(roots)) => shortcut_actions(frame, roots),
            _ => failure(&frame["id"], "unsupported"),
        }
    }

    #[must_use]
    pub fn blob_server(&self) -> Option<Arc<dyn BlobServer>> {
        self.mail_bodies
            .as_ref()
            .map(|bodies| Arc::clone(bodies) as Arc<dyn BlobServer>)
    }
}

fn shortcut_actions(frame: &Value, roots: &[PathBuf]) -> Value {
    let id = &frame["id"];
    if frame["args"]
        .as_object()
        .is_none_or(|args| !args.is_empty())
    {
        return failure(id, "invalid");
    }
    json!({
        "t":"QueryResult","id":id,
        "result_json":ab_native::shortcut_actions::discover(roots),"blobs":[]
    })
}

#[derive(Debug)]
pub struct MailBodies {
    root: PathBuf,
    held: Mutex<BTreeMap<String, (Vec<u8>, Instant)>>,
    sequence: AtomicU64,
}

fn valid_args(args: &Value) -> bool {
    args.as_object().is_some_and(|object| object.len() == 1)
        && args["message_id"].as_str().is_some_and(|id| {
            !id.is_empty() && id.len() < 20 && id.bytes().all(|b| b.is_ascii_digit())
        })
}

fn failure(id: &Value, code: &str) -> Value {
    json!({"t":"QueryError","id":id,"code":code})
}

impl MailBodies {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            held: Mutex::new(BTreeMap::new()),
            sequence: AtomicU64::new(0),
        }
    }

    pub fn answer(&self, frame: &Value) -> Value {
        let id = &frame["id"];
        if !valid_args(&frame["args"]) {
            return failure(id, "invalid");
        }
        let Ok(result) = ab_native::mail_body::read(&frame["args"], &self.root) else {
            return failure(id, "unavailable");
        };
        let Ok(encoded) = serde_json::to_vec(&result) else {
            return failure(id, "unavailable");
        };
        if encoded.len() <= INLINE_LIMIT {
            return json!({"t":"QueryResult","id":id,"result_json":result,"blobs":[]});
        }
        match self.hold(encoded) {
            Ok((digest, size, handle)) => json!({
                "t":"QueryResult","id":id,"result_json":{"body_blob":digest},
                "blobs":[{"digest":digest,"size":size,"handle":handle}]
            }),
            Err(_) => failure(id, "unavailable"),
        }
    }

    fn hold(&self, encoded: Vec<u8>) -> Result<(String, usize, String), Error> {
        let mut held = self.held.lock().map_err(|_| Error::WorkerPanicked)?;
        let now = Instant::now();
        held.retain(|_, (_, expires)| *expires > now);
        let total: usize = held.values().map(|(data, _)| data.len()).sum();
        if total + encoded.len() > HELD_LIMIT {
            return Err(Error::Conflict);
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid)?
            .as_nanos();
        let handle = format!(
            "{HANDLE}{nanos:032x}{:016x}",
            self.sequence.fetch_add(1, Ordering::Relaxed)
        );
        let digest = hex(&Sha256::digest(&encoded));
        let size = encoded.len();
        held.insert(handle.clone(), (encoded, now + HOLD));
        Ok((digest, size, handle))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

impl BlobServer for MailBodies {
    fn owns(&self, handle: &str) -> bool {
        handle.starts_with(HANDLE)
    }

    fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        let mut held = self.held.lock().map_err(|_| Error::WorkerPanicked)?;
        let (data, expires) = held.get(handle).ok_or(Error::Conflict)?;
        if *expires <= Instant::now() {
            held.remove(handle);
            return Err(Error::Conflict);
        }
        let start = usize::try_from(offset).map_err(|_| Error::Invalid)?;
        if start > data.len() {
            return Err(Error::Invalid);
        }
        let end = data.len().min(start.saturating_add(length));
        let reply = json!({
            "t":"BlobChunk","handle":handle,"offset":offset,
            "bytes":STANDARD.encode(&data[start..end]),
            "digest":hex(&Sha256::digest(data)),
            "eof":end == data.len(),
        });
        if end == data.len() {
            held.remove(handle);
        }
        Ok(reply)
    }
}
