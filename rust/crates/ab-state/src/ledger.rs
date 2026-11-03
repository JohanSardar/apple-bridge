use crate::number;
use crate::{
    Change, Connection, Cursor, Deserialize, Error, ErrorCode, Gateway, OptionalExtension, Path,
    Record, Result, Serialize, State, apply, encoded, gateway, head, params, set_cursor, set_meta,
};
use ab_domain::{CommandId, CommandState, Source};
use ab_wire::gojson::GoTime;
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(rename = "command_id")]
    pub id: CommandId,
    pub op: String,
    pub args: Box<RawValue>,
    #[serde(default, skip_serializing_if = "zero")]
    pub expected_version: u64,
    #[serde(default)]
    pub expires_at: GoTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admission_era: String,
    #[serde(default, skip_serializing_if = "zero_i64")]
    pub wait_ms: i64,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn zero(n: &u64) -> bool {
    *n == 0
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn zero_i64(n: &i64) -> bool {
    *n == 0
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPart {
    pub index: usize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub upload_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub record_id: String,
    pub disposition: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandEvent {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transition: String,
    pub state: CommandState,
    pub at: GoTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<RawValue>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobReference {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub awaiting_variants: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub variant: bool,
    pub digest: String,
    pub size: i64,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Box<Record>>,
    #[serde(default)]
    pub retryable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<CommandPart>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub upload_references: BTreeMap<String, BlobReference>,
    pub command: Command,
    pub principal: String,
    pub source: Source,
    pub request_hash: String,
    pub state: CommandState,
    pub admission_era: String,
    pub created_at: GoTime,
    pub expires_at: GoTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub disposition: String,
    #[serde(default)]
    pub commit_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_record_ids: Vec<String>,
    #[serde(default)]
    pub cursor: Cursor,
    #[serde(default)]
    pub history: Vec<CommandEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}
pub(crate) fn check_ledger(db: &Connection, home: &Path) -> Result<()> {
    if home.join("ledger.db").try_exists()? {
        return Ok(());
    }
    let prior: i64 = db.query_row(
        "SELECT count(*) FROM records WHERE collection='system.command'",
        [],
        |r| r.get(0),
    )?;
    let marked: i64 = db.query_row(
        "SELECT count(*) FROM meta WHERE key='commands_seen'",
        [],
        |r| r.get(0),
    )?;
    let legacy: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name='commands'",
        [],
        |r| r.get(0),
    )?;
    let legacy = if legacy > 0 {
        db.query_row("SELECT count(*) FROM commands", [], |r| r.get::<_, i64>(0))?
    } else {
        0
    };
    if prior > 0 || marked > 0 || legacy > 0 {
        return Err(Error::LedgerMissing);
    }
    Ok(())
}
pub(crate) fn open_ledger(home: &Path) -> Result<Connection> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = home.join("ledger.db");
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)?;
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS commands(id TEXT PRIMARY KEY,principal TEXT NOT NULL,request_hash TEXT NOT NULL,state TEXT NOT NULL,payload BLOB NOT NULL); CREATE INDEX IF NOT EXISTS commands_state ON commands(state); CREATE TABLE IF NOT EXISTS dedupe(command_id TEXT PRIMARY KEY,principal TEXT NOT NULL,request_hash TEXT NOT NULL,admission_era TEXT NOT NULL);")?;
    db.execute(
        "INSERT OR IGNORE INTO meta VALUES('admission_era',?)",
        [gateway::new_id("")?],
    )?;
    std::fs::File::open(home)?.sync_all()?;
    Ok(db)
}
fn parts_change(old: &[CommandPart], next: &[CommandPart]) -> (bool, bool) {
    if next.len() > 11 || (!old.is_empty() && old.len() != next.len()) {
        return (false, false);
    }
    let mut changed = old.len() != next.len();
    let mut ids = std::collections::HashSet::new();
    for (i, p) in next.iter().enumerate() {
        if p.index != i
            || !matches!(p.kind.as_str(), "text" | "attachment")
            || (p.kind == "text" && !p.upload_id.is_empty())
            || !matches!(
                p.disposition.as_str(),
                "observed" | "delivered" | "delivery_failed" | "indeterminate" | "failed_no_effect"
            )
            || (matches!(p.disposition.as_str(), "indeterminate" | "failed_no_effect")
                && !p.record_id.is_empty())
            || (!p.record_id.is_empty() && !ids.insert(&p.record_id))
        {
            return (false, false);
        }
        if let Some(prior) = old.get(i) {
            if prior.index != p.index
                || prior.kind != p.kind
                || prior.upload_id != p.upload_id
                || (!prior.record_id.is_empty() && prior.record_id != p.record_id)
                || (prior.disposition != p.disposition
                    && prior.disposition != "indeterminate"
                    && !(prior.disposition == "observed"
                        && matches!(p.disposition.as_str(), "delivered" | "delivery_failed")))
            {
                return (false, false);
            }
            changed |= prior != p;
        }
    }
    (changed, true)
}
#[derive(Serialize)]
struct Marker<'a> {
    command_id: &'a str,
    principal: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    awaiting_approval: bool,
}

impl State<Gateway> {
    pub(crate) fn init_commands(&mut self, home: &Path) -> Result<()> {
        check_ledger(&self.db, home)?;
        self.ledger = Some(open_ledger(home)?);
        set_meta(&self.db, "admission_era", &self.admission_era()?)?;
        for record in self.commands()? {
            let version: Option<i64> = self
                .db
                .query_row(
                    "SELECT version FROM records WHERE collection='system.command' AND id=?",
                    [record.command.id.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            if version.unwrap_or(0)
                != i64::try_from(record.history.len()).map_err(|_| ErrorCode::Capacity)?
            {
                self.publish_command(&record)?;
            }
        }
        Ok(())
    }
    pub fn admission_era(&self) -> Result<String> {
        Ok(self
            .ledger
            .as_ref()
            .ok_or(ErrorCode::Unavailable)?
            .query_row(
                "SELECT value FROM meta WHERE key='admission_era'",
                [],
                |r| r.get(0),
            )?)
    }
    pub fn command(&self, id: &str) -> Result<CommandRecord> {
        let raw: Option<Vec<u8>> = self
            .ledger
            .as_ref()
            .ok_or(ErrorCode::Unavailable)?
            .query_row("SELECT payload FROM commands WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(serde_json::from_slice(&raw.ok_or(ErrorCode::NotFound)?)?)
    }
    pub fn commands(&self) -> Result<Vec<CommandRecord>> {
        let mut stmt = self
            .ledger
            .as_ref()
            .ok_or(ErrorCode::Unavailable)?
            .prepare("SELECT payload FROM commands ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        rows.map(|r| Ok(serde_json::from_slice(&r?)?)).collect()
    }
    pub fn pending_approvals(&self) -> Result<Vec<CommandRecord>> {
        Ok(self
            .commands()?
            .into_iter()
            .filter(|r| r.state == CommandState::AwaitingApproval)
            .collect())
    }
    pub fn transition_command(
        &mut self,
        from: Option<CommandState>,
        record: CommandRecord,
    ) -> Result<CommandRecord> {
        self.persist_command(from, record, false)
    }
    pub fn update_command_parts(&mut self, record: CommandRecord) -> Result<CommandRecord> {
        self.persist_command(Some(record.state), record, true)
    }
    #[allow(clippy::too_many_lines)]
    fn persist_command(
        &mut self,
        from: Option<CommandState>,
        mut r: CommandRecord,
        parts_updated: bool,
    ) -> Result<CommandRecord> {
        if parts_updated {
            if from != Some(r.state) || r.command.op != "messages.send" {
                return Err(ErrorCode::Conflict.into());
            }
        } else if let Some(from) = from {
            from.transition(r.state)?;
        } else if !matches!(
            r.state,
            CommandState::Admitted | CommandState::AwaitingApproval
        ) {
            return Err(ErrorCode::Conflict.into());
        }
        let db = self.ledger.as_mut().ok_or(ErrorCode::Unavailable)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old: Option<Vec<u8>> = tx
            .query_row(
                "SELECT payload FROM commands WHERE id=?",
                [r.command.id.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        let old: Option<CommandRecord> = old
            .map(|raw| serde_json::from_slice(&raw))
            .transpose()
            .map_err(|_| ErrorCode::Corrupt)?;
        if let Some(old) = &old {
            if old.principal != r.principal || old.request_hash != r.request_hash {
                return Err(Error::IdReused);
            }
            if Some(old.state) != from {
                return Err(ErrorCode::Conflict.into());
            }
        } else if from.is_some() {
            return Err(ErrorCode::Conflict.into());
        }
        let (changed, valid) =
            parts_change(old.as_ref().map_or(&[], |r| r.parts.as_slice()), &r.parts);
        if !valid
            || (parts_updated
                && (!changed
                    || old
                        .as_ref()
                        .is_none_or(|old| old.disposition != r.disposition)))
        {
            return Err(ErrorCode::Conflict.into());
        }
        if from.is_none() {
            let era: String = tx.query_row(
                "SELECT value FROM meta WHERE key='admission_era'",
                [],
                |r| r.get(0),
            )?;
            if r.admission_era != era {
                return Err(ErrorCode::Expired.into());
            }
        }
        let mut cursor = head(&self.db)?;
        let mut event = CommandEvent {
            transition: String::new(),
            state: r.state,
            at: GoTime::parse(&jiff::Timestamp::now().to_string())?,
            evidence: r.evidence.clone(),
        };
        if parts_updated {
            let old = old.as_ref().ok_or(ErrorCode::Conflict)?;
            if cursor.epoch != old.cursor.epoch || cursor.sequence <= old.commit_seq {
                return Err(ErrorCode::Conflict.into());
            }
            r.commit_seq = cursor.sequence;
            event.transition = "parts_updated".into();
        }
        r.history = old.map_or_else(Vec::new, |r| r.history);
        r.history.push(event);
        cursor.sequence += 1;
        r.cursor = cursor;
        if !crate::completion::completion(&r)
            && matches!(r.state, CommandState::Observed | CommandState::Delivered)
            && (r.commit_seq == 0 || r.commit_seq >= r.cursor.sequence)
        {
            return Err(ErrorCode::Corrupt.into());
        }
        let state = serde_json::to_value(r.state)?
            .as_str()
            .unwrap_or("")
            .to_owned();
        tx.execute("INSERT INTO commands VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,payload=excluded.payload",params![r.command.id.as_str(),r.principal,r.request_hash,state,encoded(&r)?])?;
        if from.is_none() {
            tx.execute(
                "INSERT INTO dedupe VALUES(?,?,?,?)",
                params![
                    r.command.id.as_str(),
                    r.principal,
                    r.request_hash,
                    r.admission_era
                ],
            )?;
            self.db
                .execute("INSERT OR IGNORE INTO meta VALUES('commands_seen','1')", [])?;
        }
        tx.commit()?;
        self.publish_command(&r)?;
        Ok(r)
    }
    fn publish_command(&mut self, r: &CommandRecord) -> Result<()> {
        let tx = self.db.transaction()?;
        let local = r
            .principal
            .split_once('/')
            .and_then(|(parent, local)| parent.starts_with("replica:").then_some(local))
            .unwrap_or(&r.principal);
        if local.starts_with("agent:") && local.len() > 6 && !local.contains(['/', '\r', '\n']) {
            let state = serde_json::to_value(r.state)?
                .as_str()
                .unwrap_or("")
                .to_owned();
            let hash = format!("{:x}", Sha256::digest(r.command.args.get().as_bytes()));
            tx.execute(
                "DELETE FROM agent_activity WHERE at<?",
                [jiff::Timestamp::now().as_second() - 90 * 24 * 3600],
            )?;
            tx.execute("INSERT INTO agent_activity VALUES(?,?,?,?,?,?,1,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,count=agent_activity.count,argument_bytes=excluded.argument_bytes,argument_hash=excluded.argument_hash,sequence=excluded.sequence WHERE agent_activity.kind='read' OR excluded.sequence>=agent_activity.sequence",params![format!("{}:{}",r.principal,r.command.id.as_str()),r.principal,r.command.op,"command",state,r.created_at.as_str().parse::<jiff::Timestamp>().map_err(|_|ErrorCode::Invalid)?.as_second(),i64::try_from(r.command.args.get().len()).map_err(|_|ErrorCode::Capacity)?,hash,number(r.cursor.sequence)?])?;
        }
        let mut cursor = head(&tx)?;
        cursor.sequence += 1;
        let marker = encoded(&Marker {
            command_id: r.command.id.as_str(),
            principal: &r.principal,
            awaiting_approval: r.state == CommandState::AwaitingApproval,
        })?;
        let record: Record = serde_json::from_value(
            serde_json::json!({"collection":"system.command","id":r.command.id.as_str(),"version":r.history.len(),"source":r.source,"observed_at":r.history.last().ok_or(ErrorCode::Corrupt)?.at,"deleted":false,"coverage":{"fields":"complete","missing":[]},"data":serde_json::from_slice::<Value>(&marker)?}),
        )?;
        let record = Record {
            data: RawValue::from_string(String::from_utf8_lossy(&marker).into_owned())?,
            ..record
        };
        let owner = format!("command:{}", r.command.id.as_str());
        tx.execute("DELETE FROM blob_refs WHERE owner=?", [&owner])?;
        crate::projection::visit_blobs(&tx, &owner, &serde_json::to_value(r)?, 0, false, false)?;
        let mut refs = Vec::<BlobReference>::new();
        let mut stmt = tx.prepare("SELECT payload FROM blob_refs WHERE owner=? ORDER BY digest")?;
        for raw in stmt.query_map([&owner], |r| r.get::<_, Vec<u8>>(0))? {
            refs.push(serde_json::from_slice(&raw?)?);
        }
        drop(stmt);
        tx.execute("DELETE FROM blob_refs WHERE owner=?", [&owner])?;
        tx.execute("INSERT INTO blob_reference_sets VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload",params![format!("command:{}",r.command.id.as_str()),encoded(&refs)?])?;
        apply(
            &tx,
            &cursor.epoch,
            &Change {
                sequence: cursor.sequence,
                op: "upsert".into(),
                record,
            },
        )?;
        set_cursor(&tx, &cursor)?;
        crate::protection::commit(tx, self.blob_protection.as_ref())
    }
}
