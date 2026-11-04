use crate::{Error, Result, session_fixtures::python};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, GzBuilder};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const HASHTAG_UTI: &str = "com.apple.notes.inlinetextattachment.hashtag";

const SCHEMA: &str = "CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('lab-notes');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount'),(4,'ICAttachment');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT,ZALTTEXT TEXT,ZSMARTFOLDERQUERYJSON TEXT,ZCREATIONDATE1 REAL,ZMODIFICATIONDATE1 REAL);";

const WRITE: &str = "import base64,json,sqlite3,sys\nc=sqlite3.connect(sys.argv[1])\nc.executescript(sys.argv[2])\nc.executemany('INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZTITLE2,ZNAME,ZACCOUNT8,ZPARENT,ZFOLDER,ZNOTEDATA,ZMARKEDFORDELETION,ZFOLDERTYPE,ZISPASSWORDPROTECTED,ZISPINNED,Z_OPT,ZNOTE1,ZPARENTATTACHMENT,ZMEDIA,ZTYPEUTI1,ZALTTEXT,ZSMARTFOLDERQUERYJSON,ZCREATIONDATE1,ZMODIFICATIONDATE1) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',json.loads(sys.argv[3]))\nc.executemany('INSERT INTO ZICNOTEDATA VALUES(?,?,?)',[(a,base64.b64decode(b) if b else None,d) for a,b,d in json.loads(sys.argv[4])])\nc.commit()\nc.close()\n";

fn field(number: u8, wire: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(number << 3) | wire];
    if wire == 2 {
        let mut length = payload.len();
        while length >= 0x80 {
            out.push(u8::try_from(length & 0x7f).unwrap_or(0) | 0x80);
            length >>= 7;
        }
        out.push(u8::try_from(length).unwrap_or(0));
    }
    out.extend(payload);
    out
}

fn varint(number: u8, mut value: usize) -> Vec<u8> {
    let mut out = vec![number << 3];
    while value >= 0x80 {
        out.push(u8::try_from(value & 0x7f).unwrap_or(0) | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).unwrap_or(0));
    out
}

#[derive(Clone, Default)]
struct Run {
    text: String,
    weight: usize,
    underline: bool,
    strike: bool,
    attach: String,
}

#[derive(Clone)]
struct Paragraph {
    style: i32,
    checked: bool,
    id: Vec<u8>,
    runs: Vec<Run>,
}

impl Paragraph {
    fn plain() -> Self {
        Self {
            style: -1,
            checked: false,
            id: Vec::new(),
            runs: Vec::new(),
        }
    }
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn run_message(run: &Run, paragraph: Option<&Paragraph>) -> Vec<u8> {
    let mut message = varint(1, units(&run.text));
    if let Some(paragraph) = paragraph {
        let mut inner = Vec::new();
        if let Ok(style) = usize::try_from(paragraph.style) {
            inner.extend(varint(1, style));
        }
        inner.extend(varint(3, 1));
        if paragraph.style == 103 {
            let mut checklist = field(1, 2, &paragraph.id);
            checklist.extend(varint(2, usize::from(paragraph.checked)));
            inner.extend(field(5, 2, &checklist));
        }
        inner.extend(field(9, 2, &[0; 16]));
        message.extend(field(2, 2, &inner));
    }
    if run.weight > 0 {
        message.extend(varint(5, run.weight));
    }
    if run.underline {
        message.extend(varint(6, 1));
    }
    if run.strike {
        message.extend(varint(7, 1));
    }
    if !run.attach.is_empty() {
        let mut attachment = field(1, 2, run.attach.as_bytes());
        attachment.extend(field(2, 2, HASHTAG_UTI.as_bytes()));
        message.extend(field(12, 2, &attachment));
    }
    message.extend(varint(13, 1));
    message
}

fn body_from(paragraphs: &mut [Paragraph], revision: &str) -> Vec<u8> {
    let mut text = String::new();
    let mut runs = Vec::new();
    let count = paragraphs.len();
    for (index, paragraph) in paragraphs.iter_mut().enumerate() {
        let last = index + 1 == count;
        if paragraph.runs.is_empty() {
            paragraph.runs = vec![Run::default()];
        }
        let total = paragraph.runs.len();
        let owner = paragraph.clone();
        for (position, run) in paragraph.runs.iter().enumerate() {
            let mut run = run.clone();
            if position + 1 == total && !last && !run.text.ends_with('\n') {
                run.text.push('\n');
            }
            if run.text.is_empty() {
                continue;
            }
            text.push_str(&run.text);
            let message = run_message(&run, (position == 0).then_some(&owner));
            runs.extend(field(5, 2, &message));
        }
    }
    let mut note = field(2, 2, text.as_bytes());
    note.extend(field(3, 2, &varint(2, units(&text))));
    note.extend(runs);
    let mut document = varint(2, 0);
    document.extend(field(3, 2, &note));
    let raw = field(2, 2, &document);
    let comment: Vec<u8> = revision.chars().filter_map(|c| u8::try_from(u32::from(c)).ok()).collect();
    let mut encoder = GzBuilder::new()
        .comment(comment)
        .write(Vec::new(), Compression::default());
    let _ = encoder.write_all(&raw);
    encoder.finish().unwrap_or_default()
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => name
                .strip_prefix('#')
                .and_then(|digits| {
                    digits
                        .strip_prefix(['x', 'X'])
                        .map_or_else(|| digits.parse::<u32>().ok(), |hex| u32::from_str_radix(hex, 16).ok())
                })
                .and_then(char::from_u32),
        };
        if let Some(decoded) = decoded {
            out.push(decoded);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

enum Token {
    Start(String),
    End(String),
    Text(String),
}

fn tokens(source: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut rest = source;
    while !rest.is_empty() {
        let tag = rest.starts_with('<')
            && rest[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        if !tag {
            let next = rest[1..].find('<').map_or(rest.len(), |at| at + 1);
            out.push(Token::Text(unescape(&rest[..next])));
            rest = &rest[next..];
            continue;
        }
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let end = rest.find('>').map_or(rest.len(), |at| at + 1);
        let inner = rest[1..end].trim_end_matches('>');
        rest = &rest[end..];
        if inner.starts_with('!') {
            continue;
        }
        let (closing, inner) = inner.strip_prefix('/').map_or((false, inner), |inner| (true, inner));
        let name: String = inner
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/')
            .collect::<String>()
            .to_lowercase();
        out.push(if closing {
            Token::End(name)
        } else {
            Token::Start(name)
        });
    }
    out
}

fn html_paragraphs(source: &str) -> Vec<Paragraph> {
    let block = |tag: &str| matches!(tag, "div" | "p" | "h1" | "h2" | "h3" | "li");
    let mut paragraphs = Vec::new();
    let mut current = Paragraph::plain();
    let mut open = false;
    let (mut bold, mut italic, mut underline, mut strike) = (0_i32, 0_i32, 0_i32, 0_i32);
    for token in tokens(source) {
        match token {
            Token::Start(tag) => match tag.as_str() {
                tag if block(tag) => {
                    if open {
                        paragraphs.push(current);
                    }
                    current = Paragraph::plain();
                    open = true;
                }
                "b" | "strong" => bold += 1,
                "i" | "em" => italic += 1,
                "u" => underline += 1,
                "s" | "strike" => strike += 1,
                "br" if open && !current.runs.is_empty() => current.runs.push(Run {
                    text: " ".into(),
                    ..Run::default()
                }),
                _ => {}
            },
            Token::End(tag) => match tag.as_str() {
                tag if block(tag) => {
                    if open {
                        paragraphs.push(current);
                    }
                    current = Paragraph::plain();
                    open = false;
                }
                "b" | "strong" => bold -= 1,
                "i" | "em" => italic -= 1,
                "u" => underline -= 1,
                "s" | "strike" => strike -= 1,
                _ => {}
            },
            Token::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                open = true;
                let weight = usize::from(bold > 0) | (usize::from(italic > 0) << 1);
                current.runs.push(Run {
                    text,
                    weight,
                    underline: underline > 0,
                    strike: strike > 0,
                    attach: String::new(),
                });
            }
        }
    }
    if open {
        paragraphs.push(current);
    }
    if paragraphs.is_empty() {
        paragraphs.push(Paragraph::plain());
    }
    paragraphs
}

fn reference_key(value: &Value) -> String {
    value["$ref"]["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn notes_date(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map_or(Value::Null, |at| {
            #[allow(clippy::cast_precision_loss)]
            let seconds = at.as_nanosecond() as f64 / 1e9 - 978_307_200.0;
            json!(seconds)
        })
}

#[derive(Debug, Default)]
pub(crate) struct Notes {
    signature: String,
    folder_pk: BTreeMap<String, i64>,
    note_pk: BTreeMap<String, i64>,
    account_pk: BTreeMap<String, i64>,
    revisions: BTreeMap<String, i64>,
    versions: BTreeMap<String, String>,
}

impl Notes {
    fn assign(&mut self, which: u8, key: &str) -> i64 {
        let next = [&self.account_pk, &self.folder_pk, &self.note_pk]
            .iter()
            .flat_map(|table| table.values().copied())
            .max()
            .unwrap_or(0)
            + 1;
        let table = match which {
            0 => &mut self.account_pk,
            1 => &mut self.folder_pk,
            _ => &mut self.note_pk,
        };
        *table.entry(key.to_owned()).or_insert(next)
    }

    fn version(&mut self, key: &str, parts: &Value) -> i64 {
        let signature = parts.to_string();
        match self.versions.get(key) {
            None => {
                self.versions.insert(key.to_owned(), signature);
                self.revisions.insert(key.to_owned(), 1);
            }
            Some(previous) if *previous != signature => {
                self.versions.insert(key.to_owned(), signature);
                *self.revisions.entry(key.to_owned()).or_insert(0) += 1;
            }
            Some(_) => {}
        }
        self.revisions.get(key).copied().unwrap_or(1)
    }
}

fn text_of(data: &Value, key: &str) -> String {
    data[key].as_str().unwrap_or_default().to_owned()
}

#[allow(clippy::too_many_lines)]
fn write_notes(
    state: &mut Notes,
    root: &Path,
    hold: &Path,
    source: &[(String, String, bool, Value)],
) -> Result<()> {
    let mut folders: BTreeMap<&str, (&Value, bool)> = BTreeMap::new();
    let mut notes: BTreeMap<&str, (&Value, bool)> = BTreeMap::new();
    for (collection, key, deleted, data) in source {
        if collection == "notes.folder" {
            folders.insert(key, (data, *deleted));
        } else {
            notes.insert(key, (data, *deleted));
        }
    }
    let hydrating = notes
        .values()
        .any(|(data, _)| data["body_status"] == "pending");
    let outcome = (|| -> Result<bool> {
        if hydrating {
            fs::write(hold, b"")?;
        }
        let mut account_names: BTreeMap<String, String> = BTreeMap::new();
        for (data, _) in folders.values() {
            let account = text_of(data, "account");
            let name = text_of(data, "account_name");
            let entry = account_names.entry(account.clone()).or_insert(account);
            if !name.is_empty() {
                *entry = name;
            }
        }
        let mut rows: Vec<Value> = Vec::new();
        for (account, name) in &account_names {
            let pk = state.assign(0, account);
            rows.push(json!([pk, 3, account, null, null, name, null, null, null, null, 0, 0, 0, 0, 1, null, null, null, null, null, null, null, null]));
        }
        for (key, (data, deleted)) in &folders {
            let account = text_of(data, "account");
            let title = text_of(data, "title");
            let recent = data["recently_deleted"].as_bool().unwrap_or_default();
            let parent_key = reference_key(&data["parent_id"]);
            let parent = if !parent_key.is_empty() && folders.contains_key(parent_key.as_str()) {
                json!(state.assign(1, &parent_key))
            } else {
                Value::Null
            };
            let smart = if data["smart"].as_bool().unwrap_or_default() {
                json!("{}")
            } else {
                Value::Null
            };
            let revision = state.version(
                &format!("folder:{key}"),
                &json!([title, account, recent, parent, smart, deleted]),
            );
            let pk = state.assign(1, key);
            let account_pk = state.account_pk.get(&account).copied();
            rows.push(json!([pk, 1, key, null, title, null, account_pk, parent, null, null, i32::from(*deleted), i32::from(recent), 0, 0, revision, null, null, null, null, null, smart, null, null]));
        }
        let mut bodies: Vec<Value> = Vec::new();
        let first_folder = folders.keys().next().copied();
        for (key, (data, deleted)) in &notes {
            let mut folder_key = reference_key(&data["folder_id"]);
            if !folders.contains_key(folder_key.as_str()) {
                folder_key = first_folder.unwrap_or_default().to_owned();
            }
            if folder_key.is_empty() {
                continue;
            }
            let title = text_of(data, "title");
            let text = text_of(data, "plain_text");
            let markup = text_of(data, "html");
            let locked = data["locked"].as_bool().unwrap_or_default();
            let pinned = data["pinned"].as_bool().unwrap_or_default();
            let status = text_of(data, "content_status");
            let mut paragraphs: Option<Vec<Paragraph>> = None;
            if !locked && status != "offloaded" && (!text.is_empty() || !markup.is_empty()) {
                let mut built = if markup.is_empty() {
                    vec![Paragraph {
                        runs: vec![Run {
                            text,
                            ..Run::default()
                        }],
                        ..Paragraph::plain()
                    }]
                } else {
                    html_paragraphs(&markup)
                };
                for (index, block) in data["blocks"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                    if index < built.len() && block["list_style"] == "checklist" {
                        let mut identifier = key.as_bytes().to_vec();
                        identifier.resize(16, 0);
                        built[index].style = 103;
                        built[index].checked = block["checked"].as_bool().unwrap_or_default();
                        built[index].id = identifier;
                    }
                }
                paragraphs = Some(built);
            }
            let fingerprint = text_of(data, "native_fingerprint");
            let pk = state.assign(2, key);
            let mut tags: Vec<String> = Vec::new();
            if paragraphs.is_some() {
                tags = data["tags"]
                    .as_array()
                    .map(|values| values.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
            }
            for (index, tag) in tags.iter().enumerate() {
                let identifier = format!("{key}-tag-{}", char::from(b'a' + u8::try_from(index).unwrap_or(0)));
                if let Some(last) = paragraphs.as_mut().and_then(|built| built.last_mut()) {
                    last.runs.push(Run {
                        text: "\u{fffc}".into(),
                        attach: identifier.clone(),
                        ..Run::default()
                    });
                }
                let attachment = state.assign(2, &format!("attachment:{identifier}"));
                rows.push(json!([attachment, 4, identifier, null, null, null, null, null, null, null, 0, 0, 0, 0, 1, pk, null, null, HASHTAG_UTI, tag, null, null, null]));
            }
            let (body, body_pk) = match paragraphs.as_mut() {
                Some(built) => (
                    json!(STANDARD.encode(body_from(built, &fingerprint))),
                    json!(pk),
                ),
                None => (Value::Null, Value::Null),
            };
            let created = notes_date(&data["created_at"]);
            let modified = notes_date(&data["modified_at"]);
            let revision = state.version(
                &format!("note:{key}"),
                &json!([title, body, locked, pinned, folder_key, deleted, created, modified, tags]),
            );
            bodies.push(json!([pk, body, revision]));
            let folder = state.folder_pk.get(&folder_key).copied();
            rows.push(json!([pk, 2, key, title, null, null, null, null, folder, body_pk, i32::from(*deleted), 0, i32::from(locked), i32::from(pinned), revision, null, null, null, null, null, null, created, modified]));
        }
        let encoded_rows = serde_json::to_string(&rows)?;
        let encoded_bodies = serde_json::to_string(&bodies)?;
        let signature = format!("{encoded_rows}{encoded_bodies}");
        if signature == state.signature {
            return Ok(hydrating);
        }
        fs::create_dir_all(root)?;
        let temporary = root.join(".tmp-NoteStore.sqlite");
        let _ = fs::remove_file(&temporary);
        python(
            WRITE,
            &[
                &temporary.to_string_lossy(),
                SCHEMA,
                &encoded_rows,
                &encoded_bodies,
            ],
        )?;
        fs::rename(&temporary, root.join("NoteStore.sqlite"))?;
        state.signature = signature;
        Ok(hydrating)
    })();
    if !hydrating {
        let _ = fs::remove_file(hold);
    }
    outcome.map(|_| ())
}

fn inspect(directory: &Path) -> Result<Vec<Value>> {
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--max-time", "5", "--unix-socket"])
        .arg(directory.join("sim.sock"))
        .arg("http://sim/inspect")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(Error::Invalid("inspect failed".into()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

pub(crate) struct NotesSync {
    state: Arc<Mutex<Notes>>,
    directory: PathBuf,
    root: PathBuf,
    hold: PathBuf,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for NotesSync {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NotesSync")
    }
}

fn sync_once(state: &Mutex<Notes>, directory: &Path, root: &Path, hold: &Path) -> Result<()> {
    if crate::session_control_empty() {
        return Ok(());
    }
    let observations = inspect(directory)?;
    let rows: Vec<(String, String, bool, Value)> = observations
        .iter()
        .filter(|row| matches!(row["collection"].as_str(), Some("notes.folder" | "notes.note")))
        .map(|row| {
            (
                text_of(row, "collection"),
                text_of(row, "native_key"),
                row["kind"] == "absent",
                if row["data"].is_object() {
                    row["data"].clone()
                } else {
                    json!({})
                },
            )
        })
        .collect();
    let mut state = state.lock().map_err(|_| Error::Invalid("notes state".into()))?;
    write_notes(&mut state, root, hold, &rows)
}

impl NotesSync {
    pub(crate) fn start(directory: PathBuf, root: PathBuf, hold: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(Notes::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (state, stop) = (Arc::clone(&state), Arc::clone(&stop));
            let (directory, root, hold) = (directory.clone(), root.clone(), hold.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let _ = sync_once(&state, &directory, &root, &hold);
                    thread::sleep(Duration::from_millis(25));
                }
            })
        };
        Self {
            state,
            directory,
            root,
            hold,
            stop,
            handle: Some(handle),
        }
    }

    pub(crate) fn sync(&self) -> Result<()> {
        sync_once(&self.state, &self.directory, &self.root, &self.hold)
    }

    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
