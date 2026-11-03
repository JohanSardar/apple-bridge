use crate::{Result, State, Visibility, allows, records};
use rusqlite::params;
use serde_json::Value;
impl<Role> State<Role> {
    pub fn restrict(&mut self, visibility: &Visibility) -> Result<()> {
        let tx = self.db.transaction()?;
        let stored = records(
            &tx,
            "SELECT payload FROM records ORDER BY collection,id",
            &[],
        )?;
        for record in stored {
            if !allows(visibility, &record)? {
                tx.execute(
                    "DELETE FROM records WHERE collection=? AND id=?",
                    params![crate::collection(record.collection), record.id.as_str()],
                )?;
                tx.execute(
                    "DELETE FROM blob_refs WHERE owner=?",
                    [format!("record:{}", record.id.as_str())],
                )?;
            }
        }
        tx.execute_batch("DELETE FROM pending_changes; DELETE FROM staging_records; DELETE FROM blob_refs WHERE owner LIKE 'capture-stage:%' OR owner LIKE 'staging:%';
DELETE FROM msg_by_chat WHERE id NOT IN (SELECT id FROM records WHERE collection='messages.message');
DELETE FROM reaction_by_message WHERE id NOT IN (SELECT id FROM records WHERE collection='messages.reaction');
DELETE FROM attachment_by_message WHERE id NOT IN (SELECT id FROM records WHERE collection='messages.attachment');
DELETE FROM activity_by_chat WHERE message_id NOT IN (SELECT id FROM msg_by_chat) OR (id NOT IN (SELECT id FROM msg_by_chat) AND id NOT IN (SELECT id FROM reaction_by_message));
DELETE FROM chat_summary WHERE id NOT IN (SELECT id FROM records WHERE collection='messages.chat');
DELETE FROM handle_address WHERE id NOT IN (SELECT id FROM records WHERE collection='messages.handle');
DELETE FROM chat_participant WHERE chat_id NOT IN (SELECT id FROM records WHERE collection='messages.chat');")?;
        crate::commit(tx)?;
        self.pending = None;
        self.generation += 1;
        Ok(())
    }
    pub fn blob_references(&self) -> Result<Vec<Value>> {
        let tx = self.db.unchecked_transaction()?;
        let mut out = Vec::new();
        let mut stmt = tx.prepare_cached("SELECT payload FROM blob_refs ORDER BY owner,digest")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            out.push(serde_json::from_slice(&raw)?);
        }
        let mut stmt =
            tx.prepare_cached("SELECT payload FROM blob_reference_sets ORDER BY owner")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            let refs: Vec<Value> = serde_json::from_slice(&raw)?;
            out.extend(refs);
        }
        for refs in self.snapshot_refs.values() {
            out.extend(refs.clone());
        }
        Ok(out)
    }
}

impl<Role> State<Role> {
    pub fn set_replica_status(&mut self, status: Value) -> Result<()> {
        if !status.is_object() {
            return Err(crate::ErrorCode::Invalid.into());
        }
        self.replica_status = Some(status);
        Ok(())
    }
    pub(crate) fn project_replica_status(&self, visibility: &Visibility) -> Result<Option<Value>> {
        let Some(mut status) = self.replica_status.clone() else {
            return Ok(None);
        };
        let mut allowed = std::collections::HashSet::new();
        for record in records(&self.db, "SELECT payload FROM records WHERE deleted=0", &[])? {
            if allows(visibility, &record)? {
                let mut stmt = self
                    .db
                    .prepare_cached("SELECT digest FROM blob_refs WHERE owner=?")?;
                let rows = stmt.query_map([format!("record:{}", record.id.as_str())], |r| {
                    r.get::<_, String>(0)
                })?;
                for digest in rows {
                    allowed.insert(digest?);
                }
            }
        }
        let mut entries = serde_json::Map::new();
        let (mut bytes, mut backlog, mut resumed, mut corruptions) = (0i64, 0i64, 0i64, 0i64);
        if let Some(original) = status["blobs"]["entries"].as_object() {
            for (digest, entry) in original {
                if allowed.contains(digest) {
                    entries.insert(digest.clone(), entry.clone());
                    if entry["state"] == "present" {
                        bytes += entry["size"].as_i64().unwrap_or(0);
                    } else {
                        bytes += entry["received"].as_i64().unwrap_or(0);
                        backlog += 1;
                    }
                    resumed += entry["resumed_bytes"].as_i64().unwrap_or(0);
                    corruptions += entry["corruptions"].as_i64().unwrap_or(0);
                }
            }
        }
        status["blobs"] = serde_json::json!({"entries":entries,"bytes":bytes,"backlog":backlog,"resumed_bytes":resumed,"corruptions":corruptions});
        Ok(Some(status))
    }
    pub(crate) fn replica_status_result(&self, visibility: &Visibility) -> Result<Value> {
        let mut result = serde_json::json!({"records":[]});
        if visibility.get("system") != Some(&true) && visibility.get("admin") != Some(&true) {
            return Ok(result);
        }
        if let Some(status) = self.project_replica_status(visibility)? {
            result["replica"] = status.clone();
            result["records"] = serde_json::json!([{"collection":"system.replica","id":"local","version":1,"source":"system","observed_at":jiff::Timestamp::now().to_string(),"deleted":false,"coverage":{"fields":"","missing":null},"data":status}]);
        }
        Ok(result)
    }
}
