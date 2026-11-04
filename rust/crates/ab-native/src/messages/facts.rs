use super::{Reader, Result, Row};
use crate::{
    body_digest,
    plist::{self, Value as Plist},
    typedstream,
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, fs, path::Path};

const MEMBERS: &str = "SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?";
const JOINED: &str = "SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?";
const DIRECT: &str = "SELECT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT count(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC";
const VISIBLE: &str = "NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)";
const REACTIONS: &str = "SELECT guid,associated_message_type,associated_message_emoji FROM message WHERE (is_from_me=1 OR ?='1') AND associated_message_guid LIKE ?";
const UNREAD: &str = "m.is_from_me=0 AND m.is_read=0 AND COALESCE(m.associated_message_type,0)=0";

fn column(rows: &[Row], name: &str) -> Vec<String> {
    rows.iter().map(|row| row.text(name).to_owned()).collect()
}

fn last_component(path: &str) -> &str {
    path.rsplit('/').find(|part| !part.is_empty()).unwrap_or("")
}

fn body(row: &Row) -> Option<String> {
    match row.blobs.get("attributedBody") {
        Some(bytes) => typedstream::decode(bytes).text,
        None => row.values.get("text").cloned(),
    }
}

fn int(args: &Value, key: &str) -> String {
    match &args[key] {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| {
                #[allow(clippy::cast_possible_truncation)]
                number.as_f64().map(|value| value as i64)
            })
            .unwrap_or(0)
            .to_string(),
        _ => "0".to_owned(),
    }
}

fn string<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default()
}

impl Reader {
    fn message_row(&self, guid: &str) -> Result<Option<Row>> {
        let selection = "SELECT ROWID AS row_position FROM message WHERE guid=?";
        Ok(self
            .projected(selection, &[guid.to_owned()], false)?
            .into_iter()
            .next())
    }

    fn peers(&self, chat: &str) -> Result<Vec<String>> {
        Ok(column(&self.query(JOINED, &[chat.to_owned()])?, "id"))
    }

    fn maximum(&self, table: &str) -> Result<i64> {
        let rows = self.query(
            &format!("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM {table}"),
            &[],
        )?;
        Ok(rows.first().map_or(0, |row| row.int("maximum")))
    }

    pub fn account_handles(&self) -> Result<Vec<String>> {
        let mut handles = BTreeSet::new();
        for name in ["destination_caller_id", "account_login"] {
            if self.has("message", &[name]) {
                let rows = self.query(
                    &format!("SELECT DISTINCT {name} AS handle FROM message WHERE is_from_me=1 AND {name} IS NOT NULL"),
                    &[],
                )?;
                handles.extend(column(&rows, "handle"));
            }
        }
        Ok(handles.into_iter().collect())
    }

    pub fn isolation_facts(&self) -> Result<Value> {
        let participants = self.query(
            "SELECT DISTINCT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id",
            &[],
        )?;
        let chats = self.query(
            "SELECT c.guid,c.chat_identifier FROM chat c WHERE c.service_name='iMessage' AND EXISTS(SELECT 1 FROM chat_message_join j WHERE j.chat_id=c.ROWID) ORDER BY c.ROWID DESC",
            &[],
        )?;
        let mut listed = Vec::with_capacity(chats.len());
        for chat in &chats {
            let peers = column(&self.query(MEMBERS, &[chat.text("guid").to_owned()])?, "id");
            listed.push(json!({"guid": chat.text("guid"), "identifier": chat.text("chat_identifier"), "peers": peers}));
        }
        Ok(json!({
            "account_handles": self.account_handles()?,
            "participants": participants.iter().filter_map(|row| row.values.get("id")).collect::<Vec<_>>(),
            "chats": listed,
        }))
    }

    pub fn has_unread(&self, chat: &str, through: Option<i64>) -> Result<bool> {
        let (bound, bindings) = match through {
            Some(row) => ("AND m.ROWID<=?", vec![chat.to_owned(), row.to_string()]),
            None => ("", vec![chat.to_owned()]),
        };
        let sql = format!(
            "SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? {bound} AND {UNREAD} AND {VISIBLE} LIMIT 1"
        );
        Ok(!self.query(&sql, &bindings)?.is_empty())
    }

    pub fn content_digest(&self, chat: &str, through: i64) -> Result<String> {
        let rows = self.query(
            "SELECT m.*,j.chat_id AS baseline_chat_id,r.delete_date AS baseline_delete_date FROM message m JOIN (SELECT chat_id,message_id FROM chat_message_join UNION SELECT chat_id,message_id FROM chat_recoverable_message_join) j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id LEFT JOIN chat_recoverable_message_join r ON r.chat_id=j.chat_id AND r.message_id=m.ROWID WHERE c.guid=? AND m.ROWID<=? ORDER BY m.ROWID",
            &[chat.to_owned(), through.to_string()],
        )?;
        let values: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut fields: Map<String, Value> = row
                    .values
                    .iter()
                    .map(|(name, value)| (name.clone(), json!(value)))
                    .collect();
                if row.int("associated_message_type") != 0 {
                    fields.remove("is_read");
                    fields.remove("date_read");
                }
                for (name, bytes) in &row.blobs {
                    fields.insert(name.clone(), json!(body_digest(bytes)));
                }
                Value::Object(fields)
            })
            .collect();
        Ok(body_digest(&serde_json::to_vec(&values)?))
    }

    pub fn preferences_digest(&self, chat: &str) -> Result<Option<String>> {
        let rows = self.query(
            "SELECT properties FROM chat WHERE guid=?",
            &[chat.to_owned()],
        )?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let properties = match row.blobs.get("properties") {
            Some(bytes) => match plist::archive(bytes) {
                Ok(fields) => fields,
                Err(_) => return Ok(None),
            },
            None => std::collections::BTreeMap::new(),
        };
        let fields: Map<String, Value> = properties
            .iter()
            .filter(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "lastSeenMessageGuid" | "LSMD" | "numberOfTimesRespondedtoThread"
                )
            })
            .map(|(name, value)| (name.clone(), canonical(value)))
            .collect();
        Ok(Some(body_digest(&serde_json::to_vec(&Value::Object(
            fields,
        ))?)))
    }

    pub fn created_rows(&self, baseline: i64, tag: &str, filename: &str) -> Result<Vec<String>> {
        let tag = tag.to_owned();
        let echoes = self.query(
            "SELECT guid FROM message WHERE ROWID>? AND (instr(text,?)>0 OR instr(attributedBody,CAST(? AS BLOB))>0)",
            &[baseline.to_string(), tag.clone(), tag],
        )?;
        let files = self.query(
            "SELECT DISTINCT m.guid FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id WHERE m.ROWID>? AND (a.filename=? OR a.filename LIKE ?)",
            &[baseline.to_string(), filename.to_owned(), format!("%/{filename}")],
        )?;
        Ok(column(&echoes, "guid")
            .into_iter()
            .chain(column(&files, "guid"))
            .collect())
    }

    pub fn chat_of_message(&self, guid: &str) -> Result<Option<(String, i64)>> {
        let rows = self.query(
            "SELECT c.guid,c.ROWID AS row_position FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?",
            &[guid.to_owned()],
        )?;
        Ok(rows
            .first()
            .map(|row| (row.text("guid").to_owned(), row.int("row_position"))))
    }

    pub fn memberships(&self, guid: &str) -> Result<Vec<String>> {
        let rows = self.query(
            &format!("SELECT c.guid FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=? AND {VISIBLE}"),
            &[guid.to_owned()],
        )?;
        Ok(column(&rows, "guid"))
    }

    pub fn max_row(&self) -> Result<i64> {
        self.maximum("message")
    }

    pub fn max_chat_row(&self) -> Result<i64> {
        self.maximum("chat")
    }

    pub fn fact(&self, operation: &str, args: &Value) -> Result<Option<Value>> {
        let answer = match operation {
            "messages.chat_members" => {
                let chat = string(args, "chat_guid").to_owned();
                json!({
                    "handles": column(&self.query(MEMBERS, std::slice::from_ref(&chat))?, "id"),
                    "joined": self.peers(&chat)?,
                })
            }
            "messages.direct_chats" => {
                let rows = self.query(DIRECT, &[])?;
                let chats: Vec<Value> = rows
                    .iter()
                    .map(|row| json!({"guid": row.text("guid"), "id": row.text("id")}))
                    .collect();
                json!({"chats": chats})
            }
            "messages.message" => match self.message_row(string(args, "guid"))? {
                None => json!({"exists": false}),
                Some(row) => json!({
                    "exists": true,
                    "is_from_me": row.flag("is_from_me"),
                    "error": row.int("error"),
                    "capture_chat_guid": row.text("capture_chat_guid"),
                }),
            },
            "messages.max_row_id" => json!({"value": self.max_row()?}),
            "messages.rows_after" => self.rows_after(args)?,
            "messages.target" => self.target(string(args, "guid"))?,
            "messages.reactions" | "messages.reaction_rows" => {
                self.reaction_fact(operation, args)?
            }
            "messages.replies" => {
                let rows = self.query(
                    "SELECT guid FROM message WHERE ROWID>? AND is_from_me=1 AND thread_originator_guid=? AND (text=? OR instr(attributedBody,CAST(? AS BLOB))>0)",
                    &[
                        int(args, "max_row_id"),
                        string(args, "guid").to_owned(),
                        string(args, "text").to_owned(),
                        string(args, "text").to_owned(),
                    ],
                )?;
                json!({"guids": column(&rows, "guid")})
            }
            "messages.conversation"
            | "messages.conversation_messages"
            | "messages.chat_recoverable"
            | "messages.foreign_activity" => self.conversation_fact(operation, args)?,
            _ => return Ok(None),
        };
        Ok(Some(answer))
    }

    fn reaction_fact(&self, operation: &str, args: &Value) -> Result<Value> {
        let mut bindings = vec![
            if args["self_chat"] == true { "1" } else { "0" }.to_owned(),
            format!("%{}", string(args, "guid")),
        ];
        let rows = if operation == "messages.reaction_rows" {
            bindings.push(int(args, "type"));
            self.query(
                &format!("{REACTIONS} AND associated_message_type=?"),
                &bindings,
            )?
        } else {
            self.query(REACTIONS, &bindings)?
        };
        Ok(json!({"reactions": reactions(&rows)}))
    }

    fn conversation_fact(&self, operation: &str, args: &Value) -> Result<Value> {
        Ok(match operation {
            "messages.conversation" => {
                let rows = self.query(
                    "SELECT ROWID AS row_position,last_read_message_timestamp FROM chat WHERE guid=?",
                    &[string(args, "chat_guid").to_owned()],
                )?;
                match rows.first() {
                    None => json!({"exists": false}),
                    Some(row) => {
                        let unread = self.query(
                            &format!("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.is_from_me=0 AND m.is_read=0 AND {VISIBLE} LIMIT 1"),
                            &[string(args, "chat_guid").to_owned()],
                        )?;
                        json!({
                            "exists": true,
                            "row_position": row.int("row_position"),
                            "last_read_message_timestamp": row.text("last_read_message_timestamp"),
                            "unread": !unread.is_empty(),
                        })
                    }
                }
            }
            "messages.conversation_messages" => {
                let rows = self.query(
                    &format!("SELECT m.guid,m.date FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND {VISIBLE} ORDER BY m.ROWID DESC"),
                    &[string(args, "chat_guid").to_owned()],
                )?;
                let listed: Vec<Value> = rows
                    .iter()
                    .map(|row| json!({"guid": row.text("guid"), "date": row.text("date")}))
                    .collect();
                json!({"rows": listed})
            }
            "messages.chat_recoverable" => {
                let chat = string(args, "chat_guid").to_owned();
                let rows = self.query(
                    &format!(
                        "SELECT {} AS deleted_date FROM chat c WHERE c.guid=?",
                        self.deletion(true)
                    ),
                    &[chat],
                )?;
                let deleted = rows
                    .first()
                    .and_then(|row| row.text("deleted_date").parse::<f64>().ok())
                    .unwrap_or(0.0);
                json!({"recoverable": deleted > 0.0})
            }
            "messages.foreign_activity" => {
                let chat = string(args, "chat");
                let baseline = int(args, "baseline").parse().unwrap_or(0);
                let tag = string(args, "tag").to_owned();
                let fresh = self.query(
                    &format!("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.ROWID>? AND NOT (instr(COALESCE(m.text,''),?)>0 OR COALESCE(instr(m.attributedBody,CAST(? AS BLOB)),0)>0) AND {VISIBLE} LIMIT 1"),
                    &[chat.to_owned(), baseline.to_string(), tag.clone(), tag],
                )?;
                json!({
                    "foreign_unread": self.has_unread(chat, Some(baseline))?,
                    "foreign_new": !fresh.is_empty(),
                })
            }
            _ => json!({}),
        })
    }

    fn rows_after(&self, args: &Value) -> Result<Value> {
        let maximum = int(args, "maximum");
        let limit = int(args, "limit");
        let rows = self.projected(
            "SELECT ROWID AS row_position FROM message WHERE ROWID>? AND ROWID<=(SELECT COALESCE(MAX(ROWID),0) FROM message) ORDER BY ROWID LIMIT ?",
            &[maximum, limit],
            false,
        )?;
        let mut result = Vec::new();
        for row in rows.iter().filter(|row| row.flag("is_from_me")) {
            let peers = self.peers(row.text("capture_chat_guid"))?;
            let files = self.query(
                "SELECT a.filename FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?",
                &[row.text("guid").to_owned()],
            )?;
            let files: Vec<String> = column(&files, "filename")
                .iter()
                .map(|name| last_component(name).to_owned())
                .collect();
            result.push(json!({
                "guid": row.text("guid"),
                "capture_chat_guid": row.text("capture_chat_guid"),
                "service": row.text("service"),
                "error": row.int("error"),
                "is_delivered": row.flag("is_delivered"),
                "text": row.text("text"),
                "body": body(row),
                "has_body": row.blobs.contains_key("attributedBody"),
                "peers": peers,
                "attachment_names": files,
            }));
        }
        Ok(json!({"rows": result}))
    }

    fn target(&self, guid: &str) -> Result<Value> {
        let Some(row) = self.message_row(guid)? else {
            return Ok(json!({"exists": false}));
        };
        let Some(item) = self.message_observations(&row).into_iter().next() else {
            return Ok(json!({"exists": false}));
        };
        Ok(json!({
            "exists": true,
            "target": item.data,
            "row": {
                "guid": row.text("guid"),
                "capture_chat_guid": row.text("capture_chat_guid"),
                "service": row.text("service"),
                "is_from_me": row.flag("is_from_me"),
                "item_type": row.int("item_type"),
                "row_position": row.int("row_position"),
            },
        }))
    }
}

fn reactions(rows: &[Row]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            json!({
                "guid": row.text("guid"),
                "type": row.text("associated_message_type"),
                "emoji": row.text("associated_message_emoji"),
            })
        })
        .collect()
}

fn canonical(value: &Plist) -> Value {
    match value {
        Plist::Null | Plist::Uid(_) => Value::Null,
        Plist::Bool(flag) => json!(flag),
        Plist::Integer(number) => json!(number),
        Plist::Real(number) => json!(number),
        Plist::Date(seconds) => json!({"date": seconds}),
        Plist::Data(bytes) => json!({"data_digest": body_digest(bytes)}),
        Plist::String(text) => json!(text),
        Plist::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Plist::Dictionary(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), canonical(item)))
                .collect(),
        ),
    }
}

#[must_use]
pub fn pins(home: &Path) -> Option<Vec<String>> {
    let path = home.join("Library/Preferences/com.apple.messages.pinning.plist");
    let bytes = fs::read(path).ok()?;
    pinned(&bytes)
}

fn pinned(bytes: &[u8]) -> Option<Vec<String>> {
    let root = plist::decode(bytes).ok()?;
    let Plist::Array(values) = root.dictionary()?.get("pD")?.dictionary()?.get("pP")? else {
        return None;
    };
    let pins = values
        .iter()
        .map(|value| {
            value
                .string()
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Option<Vec<_>>>()?;
    (pins.len() <= 9 && pins.iter().collect::<BTreeSet<_>>().len() == pins.len()).then_some(pins)
}

#[derive(Debug, Clone)]
pub struct CleanupRow {
    pub guid: String,
    pub row: i64,
    pub chat: String,
    pub collection: String,
    pub data: Value,
    pub raw_text: String,
    pub parent: String,
    pub retracted: bool,
}

const LIVE: &str = "NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=j.chat_id)";

impl Reader {
    pub fn tagged_guids(&self) -> Result<Vec<String>> {
        let rows = self.query(
            &format!("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE {LIVE} AND (instr(m.text,'[ab-test ')>0 OR instr(m.attributedBody,CAST('[ab-test ' AS BLOB))>0) ORDER BY m.ROWID DESC"),
            &[],
        )?;
        Ok(column(&rows, "guid"))
    }

    pub fn live_guids_after(&self, baseline: i64) -> Result<Vec<String>> {
        let rows = self.query(
            &format!("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.ROWID>? AND {LIVE}"),
            &[baseline.to_string()],
        )?;
        Ok(column(&rows, "guid"))
    }

    pub fn guids_with_text_after(&self, baseline: i64, body: &str) -> Result<Vec<String>> {
        let rows = self.query(
            &format!("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.ROWID>? AND {LIVE} AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)"),
            &[baseline.to_string(), body.to_owned(), body.to_owned()],
        )?;
        Ok(column(&rows, "guid"))
    }

    pub fn attachment_names_through(&self, baseline: i64) -> Result<Vec<(String, String)>> {
        let rows = self.query(
            "SELECT a.filename,a.transfer_name FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID WHERE j.message_id<=?",
            &[baseline.to_string()],
        )?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    row.text("filename").to_owned(),
                    row.text("transfer_name").to_owned(),
                )
            })
            .collect())
    }

    pub fn attachments_after(&self, baseline: i64) -> Result<Vec<(String, i64, String)>> {
        let rows = self.query(
            "SELECT m.guid,m.ROWID AS native_row_id,a.filename,a.transfer_name FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id JOIN chat_message_join c ON c.message_id=m.ROWID WHERE m.ROWID>? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)",
            &[baseline.to_string()],
        )?;
        Ok(rows
            .iter()
            .map(|row| {
                let name = if row.text("transfer_name").is_empty() {
                    row.text("filename")
                } else {
                    row.text("transfer_name")
                };
                (
                    row.text("guid").to_owned(),
                    row.int("native_row_id"),
                    name.to_owned(),
                )
            })
            .collect())
    }

    pub fn visible_peers(&self, guid: &str) -> Result<Vec<String>> {
        let rows = self.query(
            "SELECT DISTINCT h.id FROM message m JOIN chat_message_join cm ON cm.message_id=m.ROWID JOIN chat c ON c.ROWID=cm.chat_id JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE m.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.ROWID)",
            &[guid.to_owned()],
        )?;
        Ok(column(&rows, "id"))
    }

    pub fn all_peers(&self, guid: &str) -> Result<Vec<String>> {
        let rows = self.query(
            "SELECT DISTINCT h.id FROM message m JOIN chat_message_join cm ON cm.message_id=m.ROWID JOIN chat_handle_join j ON j.chat_id=cm.chat_id JOIN handle h ON h.ROWID=j.handle_id WHERE m.guid=?",
            &[guid.to_owned()],
        )?;
        Ok(column(&rows, "id"))
    }

    pub fn chat_peers(&self, chat: &str) -> Result<Vec<String>> {
        self.peers(chat)
    }

    pub fn is_in_chat(&self, guid: &str, chat_row: i64) -> Result<bool> {
        let rows = self.query(
            &format!("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.guid=? AND j.chat_id=? AND {LIVE}"),
            &[guid.to_owned(), chat_row.to_string()],
        )?;
        Ok(!rows.is_empty())
    }

    pub fn is_live(&self, guid: &str) -> Result<bool> {
        let rows = self.query(
            &format!("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.guid=? AND {LIVE}"),
            &[guid.to_owned()],
        )?;
        Ok(!rows.is_empty())
    }

    pub fn is_live_after(&self, guid: &str, baseline: i64) -> Result<bool> {
        let rows = self.query(
            &format!("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.guid=? AND m.ROWID>? AND {LIVE}"),
            &[guid.to_owned(), baseline.to_string()],
        )?;
        Ok(!rows.is_empty())
    }

    pub fn row_of(&self, guid: &str) -> Result<Option<i64>> {
        let rows = self.query(
            "SELECT ROWID AS position FROM message WHERE guid=?",
            &[guid.to_owned()],
        )?;
        Ok(rows.first().map(|row| row.int("position")))
    }

    pub fn cleanup_row(&self, guid: &str) -> Result<Option<CleanupRow>> {
        let Some(row) = self.message_row(guid)? else {
            return Ok(None);
        };
        let Some(item) = self.message_observations(&row).into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(CleanupRow {
            guid: row.text("guid").to_owned(),
            row: row.int("row_position"),
            chat: row.text("capture_chat_guid").to_owned(),
            collection: item.collection,
            data: item.data,
            raw_text: row.text("text").to_owned(),
            parent: row
                .text("associated_message_guid")
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
            retracted: row.int("date_retracted") != 0,
        }))
    }
}
