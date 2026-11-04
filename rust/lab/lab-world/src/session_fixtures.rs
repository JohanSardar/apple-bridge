use crate::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn python(script: &str, arguments: &[&str]) -> Result<()> {
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .args(arguments)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "python3 exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub(crate) fn replace_file(path: &Path, data: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::Invalid("fixture file name".into()))?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".tmp-{name}"));
    fs::write(&temporary, data)?;
    fs::rename(temporary, path)?;
    Ok(())
}

const PHONE: &str = r#"import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-calls')")
c.execute("CREATE TABLE IF NOT EXISTS ZCALLRECORD(Z_PK INTEGER PRIMARY KEY,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER)")
c.execute("DELETE FROM ZCALLRECORD")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZCALLRECORD VALUES(1,'synthetic','Fixture',946684800.0,?,0,1,'com.apple.Telephony',1)",(100+int(sys.argv[2]),))
c.commit();c.close()
"#;

const VOICEMAIL: &str = r#"import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-voicemail')")
c.execute("CREATE TABLE IF NOT EXISTS ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER)")
c.execute("DELETE FROM ZSTOREDMESSAGE")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,'lab-voicemail-1','Synthetic',946684800.0,?,0,0,'com.apple.coretelephony',1)",(100+int(sys.argv[2]),))
c.commit();c.close()
"#;

const VOICE_MEMOS: &str = r#"import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-recordings')")
c.execute("CREATE TABLE IF NOT EXISTS ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER)")
c.execute("DELETE FROM ZCLOUDRECORDING")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZCLOUDRECORDING VALUES(1,'lab-recording-1','Synthetic',946684800.0,?,NULL,1)",(100+int(sys.argv[2]),))
c.commit();c.close()
"#;

fn media_fixture(source: &str) -> Option<(&'static str, &'static str)> {
    match source {
        "phone" => Some(("CallHistory.storedata", PHONE)),
        "voicemail" => Some(("voicemail.db", VOICEMAIL)),
        "voice_memos" => Some(("CloudRecordings.db", VOICE_MEMOS)),
        _ => None,
    }
}

#[must_use]
pub fn session_media_source(source: &str) -> bool {
    media_fixture(source).is_some() && crate::session_sources().iter().any(|name| name == source)
}

#[must_use]
pub fn session_media_version(duration: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    {
        duration as i64 - 100
    }
}

#[must_use]
pub fn session_control_empty() -> bool {
    std::env::var("AB_LAB_SESSION_CONTROL").as_deref() == Ok("empty")
}

impl World {
    pub fn stage_session_media(&self, source: &str, version: u64, present: bool) -> Result<()> {
        let (file, script) = media_fixture(source)
            .ok_or_else(|| Error::Invalid(format!("no session media fixture for {source}")))?;
        let root = self
            .session_root(source)
            .ok_or_else(|| Error::Invalid(format!("no session root for {source}")))?;
        if session_control_empty() {
            return Ok(());
        }
        let state = if present {
            version.to_string()
        } else {
            "absent".to_owned()
        };
        python(script, &[&root.join(file).to_string_lossy(), &state])
    }
}

#[derive(Debug, Default)]
pub(crate) struct Device {
    platform: String,
    events: Vec<(f64, bool, String)>,
}

pub(crate) type Devices = BTreeMap<String, Device>;

fn segb(events: &[(f64, bool, String)]) -> Vec<u8> {
    let mut data = b"SEGB-lab".to_vec();
    for (at, starting, bundle) in events {
        if bundle.is_empty() || bundle.len() > 255 {
            continue;
        }
        data.extend([0x10, 1, 0x18, u8::from(*starting), 0x21]);
        data.extend((at - 978_307_200.0).to_le_bytes());
        data.extend([0x32, u8::try_from(bundle.len()).unwrap_or(255)]);
        data.extend(bundle.as_bytes());
    }
    data
}

const PEERS: &str = "import json,sqlite3,sys\nc=sqlite3.connect(sys.argv[1])\nc.execute('CREATE TABLE DevicePeer(device_identifier TEXT,name TEXT,platform INTEGER,me INTEGER,model TEXT)')\nc.executemany('INSERT INTO DevicePeer VALUES(?,?,?,?,?)',json.loads(sys.argv[2]))\nc.commit()\nc.close()\n";

impl World {
    pub(crate) fn stage_session_usage(&self, action: &Value) -> Result<()> {
        let root = self
            .session_root("usage")
            .ok_or_else(|| Error::Invalid("no usage root".into()))?
            .to_path_buf();
        let device = action["device"].as_str().unwrap_or_default();
        if device.is_empty() {
            return Err(Error::Invalid("usage device".into()));
        }
        let mut devices = self
            .labsession
            .usage
            .lock()
            .map_err(|_| Error::Invalid("usage state".into()))?;
        let entry = devices.entry(device.to_owned()).or_insert_with(|| Device {
            platform: action["platform"].as_str().unwrap_or_default().to_owned(),
            events: Vec::new(),
        });
        for event in action["events"].as_array().cloned().unwrap_or_default() {
            entry.events.push((
                event["at"].as_f64().unwrap_or_default(),
                event["starting"].as_bool().unwrap_or_default(),
                event["bundle"].as_str().unwrap_or_default().to_owned(),
            ));
        }
        let base = root.join("streams/restricted/App.InFocus");
        for (id, device) in devices.iter() {
            let directory = if id == "local" {
                base.join("local")
            } else {
                base.join("remote").join(id)
            };
            fs::create_dir_all(&directory)?;
            replace_file(&directory.join("1"), &segb(&device.events))?;
        }
        let rows: Vec<Value> = devices
            .iter()
            .map(|(id, device)| {
                let (platform, model) = match device.platform.as_str() {
                    "mac" => (3, "Mac"),
                    "ipad" => (2, "iPad"),
                    _ => (2, "iPhone"),
                };
                json!([id, format!("Synthetic {} {id}", device.platform), platform, i32::from(id == "local"), model])
            })
            .collect();
        drop(devices);
        let directory = root.join("sync");
        fs::create_dir_all(&directory)?;
        let temporary = directory.join(".tmp-sync.db");
        let _ = fs::remove_file(&temporary);
        python(
            PEERS,
            &[
                &temporary.to_string_lossy(),
                &serde_json::to_string(&rows)?,
            ],
        )?;
        fs::rename(temporary, directory.join("sync.db"))?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct Mail {
    kinds: BTreeMap<String, String>,
    mailboxes: BTreeMap<i64, (String, String)>,
    messages: BTreeMap<i64, Message>,
}

#[derive(Debug)]
struct Message {
    mailbox: i64,
    subject: String,
    sender: String,
    received: f64,
    read: bool,
    message_id: String,
}

fn invalid(what: &str) -> Error {
    Error::Invalid(what.to_owned())
}

fn path_escape(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~$&+=:@".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl Mail {
    fn apply(&mut self, records: &[Value]) -> Result<()> {
        for record in records {
            if record["kind"] != "upsert" {
                continue;
            }
            let key = record["native_key"].as_str().unwrap_or_default();
            let data = &record["data"];
            match record["collection"].as_str().unwrap_or_default() {
                "mail.account" => {
                    self.kinds
                        .insert(key.to_owned(), data["kind"].as_str().unwrap_or_default().to_owned());
                }
                "mail.mailbox" => {
                    let id = key
                        .trim_start_matches("mailbox/")
                        .parse::<i64>()
                        .map_err(|_| invalid("mailbox key"))?;
                    self.mailboxes.insert(
                        id,
                        (
                            data["account_id"].as_str().unwrap_or_default().to_owned(),
                            data["path"].as_str().unwrap_or_default().to_owned(),
                        ),
                    );
                }
                "mail.message" => {
                    let id = key.parse::<i64>().map_err(|_| invalid("message key"))?;
                    let mailboxes = data["mailbox_ids"].as_array().cloned().unwrap_or_default();
                    let [mailbox] = mailboxes.as_slice() else {
                        return Err(invalid("message mailboxes"));
                    };
                    let mailbox = mailbox
                        .as_str()
                        .unwrap_or_default()
                        .trim_start_matches("mailbox/")
                        .parse::<i64>()
                        .map_err(|_| invalid("message mailbox"))?;
                    let received = data["received_at"]
                        .as_str()
                        .unwrap_or_default()
                        .parse::<jiff::Timestamp>()
                        .map_err(|_| invalid("received_at"))?;
                    #[allow(clippy::cast_precision_loss)]
                    let received = received.as_nanosecond() as f64 / 1e9;
                    self.messages.insert(
                        id,
                        Message {
                            mailbox,
                            subject: data["subject"].as_str().unwrap_or_default().to_owned(),
                            sender: data["from"].as_str().unwrap_or_default().to_owned(),
                            received,
                            read: data["read"].as_bool().unwrap_or_default(),
                            message_id: data["message_id"].as_str().unwrap_or_default().to_owned(),
                        },
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn rows(&self) -> Value {
        let mailboxes: Vec<Value> = self
            .mailboxes
            .iter()
            .map(|(id, (account, path))| {
                let segments: Vec<String> = path
                    .trim_start_matches('/')
                    .split('/')
                    .map(path_escape)
                    .collect();
                let kind = self.kinds.get(account).cloned().unwrap_or_default().to_lowercase();
                json!([id, format!("{kind}://{account}/{}", segments.join("/"))])
            })
            .collect();
        let messages: Vec<Value> = self
            .messages
            .iter()
            .map(|(id, m)| {
                json!([id, m.mailbox, m.subject, m.sender, m.received, i32::from(m.read), m.message_id])
            })
            .collect();
        json!({"mailboxes": mailboxes, "messages": messages})
    }
}

const MAIL: &str = r#"import json,sqlite3,sys
rows=json.load(open(sys.argv[2]))
c=sqlite3.connect(sys.argv[1])
c.execute('PRAGMA journal_mode=DELETE')
c.execute('CREATE TABLE mailboxes(url TEXT)')
c.execute('CREATE TABLE subjects(subject TEXT)')
c.execute('CREATE TABLE addresses(address TEXT)')
c.execute('CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT)')
c.executemany('INSERT INTO mailboxes(ROWID,url) VALUES(?,?)',rows['mailboxes'])
subjects={}
addresses={}
def intern(table,column,index,value):
    if value not in index:
        index[value]=len(index)+1
        c.execute('INSERT INTO %s(ROWID,%s) VALUES(?,?)'%(table,column),(index[value],value))
    return index[value]
for rowid,mailbox,subject,sender,received,read,message_id in rows['messages']:
    c.execute('INSERT INTO messages(ROWID,mailbox,subject,sender,date_received,read,message_id) VALUES(?,?,?,?,?,?,?)',(rowid,mailbox,intern('subjects','subject',subjects,subject),intern('addresses','address',addresses,sender),received,read,message_id))
c.commit()
c.close()
"#;

impl World {
    pub(crate) fn stage_session_mail(&self, action: &Value) -> Result<()> {
        let root: PathBuf = self
            .session_root("mail")
            .ok_or_else(|| Error::Invalid("no mail root".into()))?
            .to_path_buf();
        let records = action["records"].as_array().cloned().unwrap_or_default();
        let encoded = {
            let mut mail = self
                .labsession
                .mail
                .lock()
                .map_err(|_| Error::Invalid("mail state".into()))?;
            mail.apply(&records)?;
            serde_json::to_vec(&mail.rows())?
        };
        let rows = self.directory.join("session-mail-rows.json");
        fs::write(&rows, encoded)?;
        let temporary = self.directory.join(".tmp-session-mail.db");
        let _ = fs::remove_file(&temporary);
        python(
            MAIL,
            &[&temporary.to_string_lossy(), &rows.to_string_lossy()],
        )?;
        fs::rename(temporary, root.join("Envelope Index"))?;
        Ok(())
    }
}
