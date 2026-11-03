use crate::{ErrorCode, Result, State};
use rusqlite::params;
use serde_json::Value;
impl<Role> State<Role> {
    pub fn save_query_blobs(
        &mut self,
        principal: &str,
        id: &str,
        scope: &str,
        blobs: &Value,
    ) -> Result<()> {
        let now = jiff::Timestamp::now().as_second();
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO query_blobs VALUES(?,?,?,?,?)",
            params![id, principal, scope, serde_json::to_vec(blobs)?, now + 3600],
        )?;
        tx.execute("DELETE FROM blob_reference_sets WHERE owner IN (SELECT id FROM query_blobs WHERE expires<?)", [now])?;
        tx.execute("DELETE FROM query_blobs WHERE expires<?", [now])?;
        let refs = blobs
            .as_array()
            .ok_or(ErrorCode::Invalid)?
            .iter()
            .map(|b| serde_json::json!({"digest":b["digest"],"size":b["size"],"state":"present"}))
            .collect::<Vec<_>>();
        tx.execute(
            "INSERT INTO blob_reference_sets VALUES(?,?)",
            params![id, serde_json::to_vec(&refs)?],
        )?;
        crate::commit(tx)
    }
    pub fn query_blob_grant(&self, principal: &str, id: &str) -> Result<(String, Value)> {
        let (scope, payload): (String, Vec<u8>) = self
            .db
            .query_row(
                "SELECT scope,payload FROM query_blobs WHERE id=? AND principal=? AND expires>?",
                params![id, principal, jiff::Timestamp::now().as_second()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| ErrorCode::Forbidden)?;
        Ok((scope, serde_json::from_slice(&payload)?))
    }
}
