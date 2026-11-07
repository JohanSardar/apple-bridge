use crate::{
    Change, Collection, ErrorCode, Gateway, OptionalExtension, Record, Result, State, apply, head,
    params, records, set_cursor,
};
use ab_domain::{Coverage, Source};
use serde_json::Value;
use sha2::{Digest, Sha256};

impl State<Gateway> {
    pub fn ingest_health(
        &mut self,
        device: &str,
        stream: &str,
        sequence: u64,
        digest: &str,
        samples: &[(String, Value)],
        deleted: &[String],
    ) -> Result<()> {
        if device.is_empty()
            || stream.is_empty()
            || stream.len() > 160
            || sequence == 0
            || sequence > i64::MAX.cast_unsigned()
            || samples.len() + deleted.len() > 100
            || samples
                .iter()
                .any(|(id, value)| id.is_empty() || !value.is_object())
            || deleted.iter().any(String::is_empty)
        {
            return Err(ErrorCode::Invalid.into());
        }
        let tx = self.db.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS health_streams(device TEXT,stream TEXT,sequence INTEGER,digest TEXT,PRIMARY KEY(device,stream))")?;
        let sequence = i64::try_from(sequence).map_err(|_| ErrorCode::Invalid)?;
        let prior: Option<(i64, String)> = tx
            .query_row(
                "SELECT sequence,digest FROM health_streams WHERE device=? AND stream=?",
                params![device, stream],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if prior
            .as_ref()
            .is_some_and(|(seq, hash)| *seq == sequence && hash == digest)
        {
            return Ok(());
        }
        if prior.map_or(0, |(seq, _)| seq).checked_add(1) != Some(sequence) {
            return Err(ErrorCode::Conflict.into());
        }
        let mut cursor = head(&tx)?;
        for (id, sample) in samples
            .iter()
            .map(|(id, value)| (id, Some(value)))
            .chain(deleted.iter().map(|id| (id, None)))
        {
            let id = format!(
                "hsm_{:x}",
                Sha256::digest(format!("{device}\0{stream}\0{id}"))
            );
            let old = records(
                &tx,
                "SELECT payload FROM records WHERE collection='health.sample' AND id=?",
                &[&id],
            )?
            .pop();
            let data = if let Some(sample) = sample {
                let mut value = sample.clone();
                value["device_id"] = device.into();
                crate::gateway::data(&value)?
            } else if let Some(old) = &old {
                old.data.clone()
            } else {
                crate::gateway::data(&serde_json::json!({}))?
            };
            let record = Record {
                collection: Collection::HealthSample,
                id: id.try_into()?,
                version: old.as_ref().map_or(1, |row| row.version + 1),
                source: Source::Health,
                observed_at: crate::replay::now().to_string(),
                deleted: sample.is_none(),
                coverage: Coverage {
                    permissions: Default::default(),
                    pack_version: String::new(),
                    operations: Vec::new(),
                    fields: "complete".into(),
                    missing: Some(Vec::new()),
                },
                data,
                tombstone_reason: String::new(),
            };
            cursor.sequence += 1;
            apply(
                &tx,
                &cursor.epoch,
                &Change {
                    sequence: cursor.sequence,
                    op: if record.deleted { "delete" } else { "upsert" }.into(),
                    record,
                },
            )?;
        }
        tx.execute("INSERT INTO health_streams(device,stream,sequence,digest) VALUES(?,?,?,?) ON CONFLICT(device,stream) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest",params![device,stream,sequence,digest])?;
        set_cursor(&tx, &cursor)?;
        crate::protection::commit(tx, self.blob_protection.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_retry_restart_gap_and_tombstone() -> Result<()> {
        let home = std::env::temp_dir().join(crate::gateway::new_id("health")?);
        let sample = vec![(
            "sample".into(),
            serde_json::json!({"type":"steps","value":120}),
        )];
        {
            let mut state = State::<Gateway>::open(&home)?;
            state.ingest_health("phone", "steps", 1, "first", &sample, &[])?;
            let cursor = state.head()?;
            state.ingest_health("phone", "steps", 1, "first", &sample, &[])?;
            assert_eq!(state.head()?, cursor);
            assert!(
                state
                    .ingest_health("phone", "steps", 1, "altered", &sample, &[])
                    .is_err()
            );
            assert!(
                state
                    .ingest_health("phone", "steps", 3, "gap", &sample, &[])
                    .is_err()
            );
            assert_eq!(state.head()?, cursor);
        }
        {
            let mut state = State::<Gateway>::open(&home)?;
            state.ingest_health("phone", "steps", 2, "deleted", &[], &["sample".into()])?;
            let rows = records(
                &state.db,
                "SELECT payload FROM records WHERE collection='health.sample'",
                &[],
            )?;
            assert_eq!(rows.len(), 1);
            assert!(rows[0].deleted);
            assert_eq!(rows[0].version, 2);
            let data: Value = serde_json::from_str(rows[0].data.get())?;
            assert_eq!(data["value"], 120);
            assert_eq!(data["device_id"], "phone");
            state.ingest_health("other", "steps", 1, "first", &sample, &[])?;
            assert_eq!(
                records(
                    &state.db,
                    "SELECT payload FROM records WHERE collection='health.sample'",
                    &[]
                )?
                .len(),
                2
            );
        }
        std::fs::remove_dir_all(home)?;
        Ok(())
    }
}
