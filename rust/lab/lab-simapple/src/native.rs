use super::{
    BTreeMap, Duration, Instant, Request, Result, Session, Shared, Value, calendar, contacts,
    generation_of, invalid, json, lock, mpsc, string, thread,
};
use lab_wire::receive;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{Arc, Mutex},
};

const FACTS_FRAME_LIMIT: usize = 262_144;
const READ_LIMIT: usize = 1 << 20;
const MAX_REVALIDATE: usize = 500;
const JOB_WAIT: Duration = Duration::from_secs(60);

const REVALIDATION: [(&str, &str); 6] = [
    ("agenda.calendars", "calendar.calendar"),
    ("agenda.events", "calendar.event"),
    ("agenda.reminder_lists", "reminders.list"),
    ("agenda.reminders", "reminders.reminder"),
    ("contacts.groups", "contacts.group"),
    ("contacts.contacts", "contacts.contact"),
];

const FACTS: [(&str, &str, &str); 7] = [
    ("agenda.calendars", "calendar", "calendar.calendar"),
    ("agenda.events", "calendar", "calendar.event"),
    ("agenda.reminder_lists", "reminders", "reminders.list"),
    ("agenda.reminders", "reminders", "reminders.reminder"),
    ("contacts.containers", "contacts", "contacts.container"),
    ("contacts.groups", "contacts", "contacts.group"),
    ("contacts.contacts", "contacts", "contacts.contact"),
];

#[derive(Debug)]
pub(super) struct Job {
    id: String,
    op: String,
    args: Value,
    command_id: String,
    ticket: String,
    reply: mpsc::Sender<Option<Value>>,
}

struct Link {
    stream: Mutex<UnixStream>,
}

impl Link {
    fn write(&self, frame: &Value) -> Result<()> {
        write_frame(
            &mut *self.stream.lock().map_err(|_| invalid("link"))?,
            frame,
        )
    }
}

static LINKS: Mutex<Vec<Arc<Link>>> = Mutex::new(Vec::new());

fn write_frame(stream: &mut UnixStream, frame: &Value) -> Result<()> {
    let body = serde_json::to_vec(frame)?;
    let size = u32::try_from(body.len()).map_err(|_| invalid("native frame"))?;
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend(body);
    stream.write_all(&bytes)?;
    Ok(())
}

fn read_frame(stream: &mut UnixStream) -> Result<Value> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > READ_LIMIT {
        return Err(invalid("native frame size"));
    }
    let mut body = vec![0_u8; size];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

pub(super) fn changed(source: &str) {
    if !matches!(source, "calendar" | "contacts" | "reminders") {
        return;
    }
    let links = LINKS.lock().map(|links| links.clone()).unwrap_or_default();
    let frame = json!({"t":"Changed","source":source,"generation":"native/1"});
    for link in links {
        let frame = frame.clone();
        thread::spawn(move || {
            let _ = link.write(&frame);
        });
    }
}

pub(super) fn serve(path: &str, state: &Shared, requests: &mpsc::Sender<Request>) -> Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(Path::new(path), std::fs::Permissions::from_mode(0o600))?;
    let (state, requests) = (Arc::clone(state), requests.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            let (state, requests) = (Arc::clone(&state), requests.clone());
            thread::spawn(move || {
                if let Err(error) = connection(stream, &state, &requests) {
                    eprintln!("native: {error}");
                }
            });
        }
    });
    Ok(())
}

struct Snapshot {
    operation: String,
    items: Vec<Value>,
    scope: Value,
    generation: String,
    expires: Instant,
}

#[derive(Default)]
struct Facts {
    fence: u64,
    counter: u64,
    subscribed: bool,
    snapshots: BTreeMap<String, Snapshot>,
}

fn connection(stream: UnixStream, state: &Shared, requests: &mpsc::Sender<Request>) -> Result<()> {
    let mut reader = stream.try_clone()?;
    let link = Arc::new(Link {
        stream: Mutex::new(stream),
    });
    let mut facts = Facts::default();
    let result = (|| -> Result<()> {
        loop {
            let frame = read_frame(&mut reader)?;
            let kind = string(&frame, "t");
            let id = string(&frame, "id");
            match kind.as_str() {
                "Hello" => {
                    facts.fence = frame["owner_fence"].as_u64().unwrap_or_default();
                    link.write(&json!({"t":"Welcome","protocol":"native/1","version":1}))?;
                }
                "Facts" => {
                    if !facts.subscribed {
                        facts.subscribed = true;
                        LINKS
                            .lock()
                            .map_err(|_| invalid("links"))?
                            .push(Arc::clone(&link));
                    }
                    page(&link, &mut facts, state, &frame)?;
                }
                "Revalidate" => revalidate(&link, state, &frame)?,
                "Effect" => effect(&link, requests, &frame),
                _ => link.write(&json!({"t":"Error","id":id,"code":"unsupported"}))?,
            }
        }
    })();
    if let Ok(mut links) = LINKS.lock() {
        links.retain(|other| !Arc::ptr_eq(other, &link));
    }
    result
}

fn failure(link: &Link, id: &str, code: &str) -> Result<()> {
    link.write(&json!({"t":"Error","id":id,"code":code}))
}

fn effect(link: &Arc<Link>, requests: &mpsc::Sender<Request>, frame: &Value) {
    let (reply, wait) = mpsc::channel();
    let job = Job {
        id: string(frame, "id"),
        op: string(frame, "op"),
        args: frame["args"].clone(),
        command_id: string(&frame["permit"], "command_id"),
        ticket: string(&frame["permit"], "ticket"),
        reply,
    };
    let identity = job.id.clone();
    let (link, requests) = (Arc::clone(link), requests.clone());
    thread::spawn(move || {
        let sent = requests.send(Request {
            frame: Value::Null,
            done: None,
            whole: false,
            exact: false,
            revision: None,
            native: Some(job),
        });
        let answer = if sent.is_ok() {
            wait.recv_timeout(JOB_WAIT).ok().flatten()
        } else {
            None
        };
        let _ = match answer {
            Some(evidence) => {
                link.write(&json!({"t":"EffectResult","id":identity,"evidence":evidence}))
            }
            None => failure(&link, &identity, "unavailable"),
        };
    });
}

fn facts_of(
    state: &Shared,
    operation: &str,
    fence: u64,
) -> std::result::Result<Snapshot, &'static str> {
    let Some((_, source, collection)) = FACTS.iter().find(|(name, _, _)| *name == operation) else {
        return Err("unsupported");
    };
    let s = lock(state).map_err(|_| "unavailable")?;
    if s.denied.get(*source) == Some(&true) {
        return Err("permission_missing");
    }
    let capture = if *source == "contacts" {
        contacts::snapshot(&s, false)
    } else {
        s.scan(source)
    }
    .map_err(|_| "unavailable")?;
    let generation = generation_of(&s.data, source, &string(&s.data, "generation"));
    drop(s);
    let mut items: Vec<Value> = capture["observations"]
        .as_array()
        .map(|records| {
            records
                .iter()
                .filter(|record| {
                    record["collection"] == *collection
                        && record["kind"] != "absent"
                        && record["kind"] != "evict"
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if operation == "contacts.contacts" {
        let mut digests: Vec<&String> = capture["blob_handles"]
            .as_object()
            .map(|handles| handles.keys().collect())
            .unwrap_or_default();
        digests.sort();
        for digest in digests {
            items.push(
                json!({"kind":"blob_handle","digest":digest,"handle":format!("{fence}:{digest}")}),
            );
        }
    }
    Ok(Snapshot {
        operation: operation.to_owned(),
        items,
        scope: json!({"collections":[collection],"history_token":""}),
        generation,
        expires: Instant::now() + Duration::from_secs(60),
    })
}

fn page(link: &Link, session: &mut Facts, state: &Shared, frame: &Value) -> Result<()> {
    let id = string(frame, "id");
    let operation = string(frame, "op");
    let limit = match frame.get("limit") {
        None | Some(Value::Null) => Some(100),
        Some(raw) => raw.as_u64().filter(|limit| *limit <= 1000),
    };
    let args_empty = frame["args"]
        .as_object()
        .is_none_or(serde_json::Map::is_empty);
    let Some(limit) = limit.filter(|limit| *limit >= 1) else {
        return failure(link, &id, "invalid");
    };
    if id.is_empty() || !args_empty || session.fence < 1 {
        return failure(link, &id, "invalid");
    }
    let limit = usize::try_from(limit).map_err(|_| invalid("limit"))?;
    let now = Instant::now();
    session
        .snapshots
        .retain(|_, snapshot| snapshot.expires > now);
    let token;
    let mut offset = 0;
    if frame["snapshot"].is_null() {
        let snapshot = match facts_of(state, &operation, session.fence) {
            Ok(snapshot) => snapshot,
            Err(code) => return failure(link, &id, code),
        };
        session.counter += 1;
        token = format!("sim-snapshot-{}", session.counter);
        session.snapshots.insert(token.clone(), snapshot);
    } else {
        token = string(frame, "snapshot");
        let cursor = string(frame, "cursor");
        let Some(stored) = session.snapshots.get(&token) else {
            return failure(link, &id, "expired");
        };
        let position = cursor
            .strip_prefix(&format!("{token}:"))
            .and_then(|rest| rest.parse::<usize>().ok());
        match position {
            Some(position) if stored.operation == operation && position <= stored.items.len() => {
                offset = position;
            }
            _ => return failure(link, &id, "invalid"),
        }
    }
    let Some(snapshot) = session.snapshots.get(&token) else {
        return failure(link, &id, "expired");
    };
    let mut end = snapshot.items.len().min(offset + limit);
    let page = loop {
        let terminal = end == snapshot.items.len();
        let page = json!({"t":"FactPage","id":id,"op":operation,"snapshot":token,"items":snapshot.items[offset..end],"cursor":if terminal { Value::Null } else { json!(format!("{token}:{end}")) },"complete":terminal,"scope":snapshot.scope,"generation":snapshot.generation});
        if serde_json::to_vec(&page)?.len() <= FACTS_FRAME_LIMIT || end <= offset + 1 {
            break page;
        }
        end = offset + (end - offset) / 2;
    };
    if end == snapshot.items.len() {
        session.snapshots.remove(&token);
    }
    link.write(&page)
}

fn revalidate(link: &Link, state: &Shared, frame: &Value) -> Result<()> {
    let id = string(frame, "id");
    let operation = string(frame, "op");
    let Some((_, collection)) = REVALIDATION.iter().find(|(name, _)| *name == operation) else {
        return failure(link, &id, "unsupported");
    };
    let identities = frame["args"]["identities"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if id.is_empty()
        || identities.is_empty()
        || identities.len() > MAX_REVALIDATE
        || identities.iter().any(|identity| {
            identity["collection"] != *collection || string(identity, "native_key").is_empty()
        })
    {
        return failure(link, &id, "invalid");
    }
    let control = std::env::var("AB_LAB_SESSION_REVALIDATE_CONTROL").unwrap_or_default();
    if control == "refuse" {
        return failure(link, &id, "unavailable");
    }
    let s = lock(state)?;
    let mut results: Vec<Value> = identities
        .iter()
        .map(|identity| {
            let key = string(identity, "native_key");
            let record = &s.data["records"][&key];
            let state = if record["kind"].is_string()
                && record["kind"] != "absent"
                && record["collection"] == *collection
            {
                if calendar::in_window(&s, record) {
                    "present"
                } else {
                    "outside_scope"
                }
            } else {
                "missing"
            };
            json!({"collection":identity["collection"],"native_key":key,"state":state})
        })
        .collect();
    drop(s);
    if control == "partial" {
        results.pop();
    }
    link.write(&json!({"t":"Revalidated","id":id,"op":operation,"results":results}))
}

impl Session<'_> {
    pub(super) fn native_job(&mut self, state: &Shared, job: &Job) -> Result<()> {
        let evidence = self.native_effect(state, job);
        let (answer, failed) = match evidence {
            Ok(evidence) => (evidence, None),
            Err(error) => (None, Some(error)),
        };
        let _ = job.reply.send(answer);
        failed.map_or(Ok(()), Err)
    }

    fn native_effect(&mut self, state: &Shared, job: &Job) -> Result<Option<Value>> {
        if std::env::var("AB_LAB_SESSION_EFFECTS_CONTROL").as_deref() == Ok("refuse") {
            return Ok(None);
        }
        let query = job.args["session_query"] == true;
        let frame = if query {
            json!({"t":"Query","id":job.id,"op":job.op,"args":job.args["args"],"deadline_ms":30000})
        } else {
            json!({"t":"Dispatch","command_id":job.command_id,"ticket":job.ticket,"owner_fence":self.fence,"origin":job.args["origin"]})
        };
        let (writer, mut reader) = UnixStream::pair()?;
        let collector = thread::spawn(move || {
            let mut frames = Vec::new();
            while let Ok((frame, _)) = receive(&mut reader) {
                frames.push(frame);
            }
            frames
        });
        let real = std::mem::replace(&mut self.stream, writer);
        let previous = self.real.replace(real);
        let result = self.command(state, frame);
        if let Some(real) = std::mem::replace(&mut self.real, previous) {
            drop(std::mem::replace(&mut self.stream, real));
        }
        let frames = collector.join().map_err(|_| invalid("collector"))?;
        result?;
        for reply in frames {
            match reply["t"].as_str() {
                Some("QueryResult") if query => {
                    let blobs = if reply["blobs"].is_array() {
                        reply["blobs"].clone()
                    } else {
                        json!([])
                    };
                    return Ok(Some(
                        json!({"observed":true,"native_query_result":reply["result_json"],"native_query_blobs":blobs}),
                    ));
                }
                Some("QueryError") if query => {
                    return Ok(Some(
                        json!({"observed":false,"native_query_error":reply["code"]}),
                    ));
                }
                Some("Outcome") if !query => {
                    let mut fields = reply["evidence"].as_object().cloned().unwrap_or_default();
                    fields.insert("disposition".into(), reply["disposition"].clone());
                    fields.insert("fingerprint".into(), job.args["request_hash"].clone());
                    return Ok(Some(Value::Object(fields)));
                }
                Some("Rejected") if !query => {
                    let mut rejected = json!({"code":"unavailable","message":"unavailable"});
                    if reply["error"].is_object() {
                        rejected["code"] = reply["error"]["code"].clone();
                        rejected["message"] = reply["error"]["message"].clone();
                    }
                    if reply["evidence"].is_object() {
                        rejected["evidence"] = reply["evidence"].clone();
                    }
                    return Ok(Some(json!({"rejected":rejected})));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn divert<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let Some(real) = self.real.take() else {
            return run(self);
        };
        let diverted = std::mem::replace(&mut self.stream, real);
        let result = run(self);
        self.real = Some(std::mem::replace(&mut self.stream, diverted));
        result
    }
}
