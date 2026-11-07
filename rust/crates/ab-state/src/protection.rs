use crate::{Connection, ErrorCode, HashMap, Result, Transaction, gateway};
use serde_json::Value;
use std::sync::Arc;

pub type ReferenceSet = HashMap<String, Vec<Value>>;
pub trait BlobProtection: std::fmt::Debug + Send + Sync {
    fn delta(&self, references: &ReferenceSet, committed: bool) -> Result<()>;
}
pub(crate) fn owner(db: &Connection, name: &str) -> Result<Vec<Value>> {
    let mut refs = Vec::new();
    let mut stmt = db.prepare("SELECT payload FROM blob_refs WHERE owner=? ORDER BY digest")?;
    for row in stmt.query_map([name], |r| r.get::<_, Vec<u8>>(0))? {
        refs.push(serde_json::from_slice(&row?)?);
    }
    let mut stmt = db.prepare("SELECT payload FROM blob_reference_sets WHERE owner=?")?;
    for row in stmt.query_map([name], |r| r.get::<_, Vec<u8>>(0))? {
        refs.extend(serde_json::from_slice::<Vec<Value>>(&row?)?);
    }
    Ok(refs)
}
pub(crate) fn commit(tx: Transaction<'_>, hook: Option<&Arc<dyn BlobProtection>>) -> Result<()> {
    let Some(hook) = hook else {
        return crate::commit(tx);
    };
    let mut refs = ReferenceSet::new();
    let mut stmt = tx.prepare("SELECT owner FROM blob_ref_changes ORDER BY owner")?;
    for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let name = row?;
        refs.insert(name.clone(), owner(&tx, &name)?);
    }
    drop(stmt);
    tx.execute("DELETE FROM blob_ref_changes", [])?;
    let all: Vec<_> = refs.values().flatten().cloned().collect();
    let guard = if all.is_empty() {
        None
    } else {
        Some(format!("transaction:{}", gateway::new_id("")?))
    };
    if let Some(guard) = &guard {
        hook.delta(&HashMap::from([(guard.clone(), all)]), false)?;
    }
    if let Err(error) = tx.commit() {
        if let Some(guard) = guard {
            let _ = hook.delta(&HashMap::from([(guard, Vec::new())]), false);
        }
        return Err(error.into());
    }
    if let Some(guard) = guard {
        refs.insert(guard, Vec::new());
    }
    hook.delta(&refs, true)
}
impl crate::State<crate::Gateway> {
    #[cfg(feature = "lab")]
    pub fn lab_command_blobs(&mut self, owner: &str, digest: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        let owner = format!("command:{owner}");
        if digest.is_empty() {
            tx.execute("DELETE FROM blob_reference_sets WHERE owner=?", [&owner])?;
        } else {
            let refs = serde_json::json!([{"digest":digest}]);
            tx.execute("INSERT INTO blob_reference_sets VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload", rusqlite::params![owner, serde_json::to_vec(&refs)?])?;
        }
        commit(tx, self.blob_protection.as_ref())
    }
    pub fn set_blob_protection(&mut self, hook: Arc<dyn BlobProtection>) -> Result<()> {
        let mut refs = ReferenceSet::new();
        let mut stmt = self
            .db
            .prepare("SELECT owner FROM blob_refs UNION SELECT owner FROM blob_reference_sets")?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            let name = row?;
            refs.insert(name.clone(), owner(&self.db, &name)?);
        }
        for (id, list) in &self.snapshot_refs {
            refs.insert(format!("snapshot:{id}"), list.clone());
        }
        hook.delta(&refs, true)?;
        drop(stmt);
        self.db.execute("DELETE FROM blob_ref_changes", [])?;
        self.blob_protection = Some(hook);
        Ok(())
    }
    pub fn source_state(
        &mut self,
        source: ab_domain::Source,
        status: &str,
        coverage: ab_domain::Coverage,
        capabilities: Option<&[String]>,
        reason: &str,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let capture = ab_wire::messages::Capture {
            source: Some(source),
            generation: "source-state".into(),
            ..Default::default()
        };
        let native = serde_json::to_value(source)?
            .as_str()
            .ok_or(ErrorCode::Invalid)?
            .to_owned();
        let observation = ab_wire::messages::Observation {
            collection: "system.source".into(),
            native_key: native.clone(),
            ..Default::default()
        };
        let id = crate::gateway::resolve(&tx, &capture, &observation, true)?
            .ok_or(ErrorCode::Corrupt)?;
        let old = crate::records(
            &tx,
            "SELECT payload FROM records WHERE collection='system.source' AND id=?",
            &[&id],
        )?
        .pop();
        let fields = std::collections::BTreeMap::from([
            ("source", serde_json::value::to_raw_value(&source)?),
            ("state", serde_json::value::to_raw_value(status)?),
            ("coverage", serde_json::value::to_raw_value(&coverage)?),
            (
                "capabilities",
                serde_json::value::to_raw_value(&capabilities)?,
            ),
            ("reason", serde_json::value::to_raw_value(reason)?),
        ]);
        let data = serde_json::value::RawValue::from_string(
            String::from_utf8_lossy(&crate::gojson::legacy_json(
                &fields,
                crate::EscapeMode::Html,
            )?)
            .into_owned(),
        )?;
        if old.as_ref().is_some_and(|r| r.data.get() == data.get()) {
            return Ok(());
        }
        let record = ab_domain::Record {
            collection: ab_domain::Collection::SystemSource,
            id: id.try_into()?,
            version: old.map_or(1, |r| r.version + 1),
            source,
            observed_at: crate::replay::now().to_string(),
            deleted: false,
            coverage,
            data,
            tombstone_reason: String::new(),
        };
        let mut cursor = crate::head(&tx)?;
        cursor.sequence += 1;
        crate::apply(
            &tx,
            &cursor.epoch,
            &crate::Change {
                sequence: cursor.sequence,
                op: "upsert".into(),
                record,
            },
        )?;
        crate::set_cursor(&tx, &cursor)?;
        commit(tx, self.blob_protection.as_ref())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_state_preserves_go_coverage_bytes_and_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let home =
            std::env::temp_dir().join(format!("source-bytes-{}", crate::gateway::new_id("")?));
        let mut state = crate::State::<crate::Gateway>::open(&home)?;
        let coverage = ab_domain::Coverage {
            permissions: Default::default(),
            pack_version: String::new(),
            operations: Vec::new(),
            fields: "full".into(),
            missing: None,
        };
        state.source_state(
            ab_domain::Source::Contacts,
            "ready",
            coverage.clone(),
            Some(&[]),
            "<ready>",
        )?;
        let payload: Vec<u8> = state.db.query_row(
            "SELECT payload FROM records WHERE collection='system.source'",
            [],
            |row| row.get(0),
        )?;
        let record: ab_domain::Record = serde_json::from_slice(&payload)?;
        assert_eq!(
            record.data.get(),
            r#"{"capabilities":[],"coverage":{"fields":"full","missing":null},"reason":"\u003cready\u003e","source":"contacts","state":"ready"}"#
        );
        let head = state.head()?;
        drop(state);
        let mut state = crate::State::<crate::Gateway>::open(&home)?;
        state.source_state(
            ab_domain::Source::Contacts,
            "ready",
            coverage,
            Some(&[]),
            "<ready>",
        )?;
        assert_eq!(state.head()?.sequence, head.sequence);
        let version: i64 = state.db.query_row(
            "SELECT version FROM records WHERE collection='system.source'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(version, 1);
        state.source_state(
            ab_domain::Source::Contacts,
            "ready",
            ab_domain::Coverage {
                permissions: Default::default(),
                pack_version: String::new(),
                operations: Vec::new(),
                fields: "full".into(),
                missing: None,
            },
            None,
            "<ready>",
        )?;
        let payload: Vec<u8> = state.db.query_row(
            "SELECT payload FROM records WHERE collection='system.source'",
            [],
            |row| row.get(0),
        )?;
        let record: ab_domain::Record = serde_json::from_slice(&payload)?;
        assert_eq!(record.version, 2);
        assert!(
            record
                .data
                .get()
                .starts_with(r#"{"capabilities":null,"coverage":{"fields":"full","missing":null}"#)
        );
        drop(state);
        std::fs::remove_dir_all(home)?;
        Ok(())
    }
}
