use crate::{Replica, Result, State, Visibility, allows, encoded, project, records};
use rusqlite::params;
use serde_json::Value;
impl State<Replica> {
    pub fn restrict(&mut self, visibility: &Visibility) -> Result<()> {
        let tx = self.db.transaction()?;
        let stored = records(
            &tx,
            "SELECT payload FROM records ORDER BY collection,id",
            &[],
        )?;
        for record in stored {
            if allows(visibility, &record)? {
                let projected = project(record.clone(), visibility)?;
                if projected.data.get() != record.data.get() {
                    tx.execute(
                        "UPDATE records SET payload=? WHERE collection=? AND id=?",
                        params![
                            encoded(&projected)?,
                            crate::collection(record.collection),
                            record.id.as_str()
                        ],
                    )?;
                }
            } else {
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
        tx.commit()?;
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
        Ok(out)
    }
}
