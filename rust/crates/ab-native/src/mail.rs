use crate::{
    capture::{self, Context, Error, Observation, Result, Snapshot},
    mail_url,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, time::Duration};
const GENERATION: &str = "SELECT 'mail-v10'";
pub fn coverage() -> Value {
    json!({"fields":"partial","missing":["account settings, recipients, attachment files and local body availability require qualification"]})
}
fn projection(
    reader: &Snapshot,
    rows: Vec<capture::Row>,
    context: &Context,
) -> Result<Vec<Observation>> {
    let mailboxes=reader.query("SELECT CAST(b.ROWID AS TEXT) AS native_key,b.url,COUNT(m.ROWID) AS message_count,SUM(CASE WHEN m.read=0 THEN 1 ELSE 0 END) AS unread_count FROM mailboxes b LEFT JOIN messages m ON m.mailbox=b.ROWID GROUP BY b.ROWID ORDER BY b.ROWID",&[],100_000,Duration::from_secs(5))?;
    let mut accounts = BTreeSet::new();
    let mut observations = Vec::new();
    for row in mailboxes {
        let parsed = mail_url::parse(
            row.get("url")
                .and_then(capture::Cell::text)
                .ok_or(Error::Corrupt)?,
        )?;
        if accounts.insert(parsed.account.clone()) {
            observations.push(capture::upsert(
                "mail.account",
                &parsed.account,
                None,
                json!({"name":parsed.account,"kind":parsed.kind,"addresses":[parsed.account]}),
                context,
            )?);
        }
        let lower = parsed.name.to_lowercase();
        let role =
            if ["inbox", "sent", "drafts", "trash", "archive", "junk"].contains(&lower.as_str()) {
                lower.as_str()
            } else {
                "other"
            };
        observations.push(capture::upsert("mail.mailbox",&format!("mailbox/{}",capture::text(&row,"native_key")),None,json!({"account_id":parsed.account,"name":parsed.name,"path":parsed.path,"role":role,"unread_count":capture::number(&row,"unread_count"),"message_count":capture::number(&row,"message_count")}),context)?);
    }
    for row in rows {
        let parsed = mail_url::parse(
            row.get("url")
                .and_then(capture::Cell::text)
                .ok_or(Error::Corrupt)?,
        )?;
        let key = capture::text(&row, "native_key");
        let alias = capture::text(&row, "message_id");
        let n = capture::number(&row, "mailbox");
        if !n.is_finite()
            || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&n)
        {
            return Err(Error::Corrupt);
        }
        #[allow(clippy::cast_possible_truncation)]
        let mailbox = n as i64;
        let data = json!({"subject":capture::text(&row,"subject"),"from":capture::text(&row,"sender"),"received_at":capture::timestamp(capture::number(&row,"date_received"))?,"read":capture::number(&row,"read")!=0.0,"message_id":alias,"mailbox_ids":[format!("mailbox/{mailbox}")],"account_id":parsed.account,"to":null,"cc":null,"sent_at":null,"flagged":flag(&row),"flag_color":flag_color(&row),"has_attachments":null,"size":null,"snippet":null,"body_state":null});
        let alias = if alias.starts_with('<') && alias.ends_with('>') && alias.contains('@') {
            Some(format!("mail/{}/{alias}", parsed.account))
        } else {
            None
        };
        observations.push(capture::upsert("mail.message", key, alias, data, context)?);
    }
    Ok(observations)
}
fn flag(row: &capture::Row) -> Option<bool> {
    (capture::number(row, "flagged") > 0.0).then_some(true)
}
fn flag_color(row: &capture::Row) -> Option<f64> {
    flag(row).map(|_| capture::number(row, "flag_color"))
}
#[derive(Debug, Clone, Default)]
struct State {
    generation: String,
    keys: BTreeSet<String>,
    material: Vec<String>,
    revision: u64,
}
#[derive(Debug, Default)]
pub struct Reader {
    state: State,
}
#[derive(Debug, serde::Serialize)]
pub struct Pending {
    #[serde(flatten)]
    pub batch: capture::Batch,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    next: State,
}
impl Reader {
    pub fn snapshot(
        &self,
        path: &Path,
        context: &Context,
        max_rows: usize,
        update_state: bool,
    ) -> Result<Pending> {
        let reader = Snapshot::open(path, GENERATION)?;
        reader.require(&[
            (
                "messages",
                &[
                    "mailbox",
                    "subject",
                    "sender",
                    "date_received",
                    "read",
                    "message_id",
                ],
            ),
            ("mailboxes", &["url"]),
            ("subjects", &["subject"]),
            ("addresses", &["address"]),
        ])?;
        let flags = if reader.has_column("messages", "flagged")?
            && reader.has_column("messages", "flag_color")?
        {
            ", m.flagged, m.flag_color"
        } else {
            ""
        };
        let rows=reader.query_partial(&format!("SELECT CAST(m.ROWID AS TEXT) AS native_key, m.mailbox, b.url, coalesce(s.subject,'') AS subject, coalesce(a.address,'') AS sender, m.date_received, m.read, coalesce(m.message_id,'') AS message_id{flags} FROM messages m LEFT JOIN mailboxes b ON b.ROWID=m.mailbox LEFT JOIN subjects s ON s.ROWID=m.subject LEFT JOIN addresses a ON a.ROWID=m.sender ORDER BY m.ROWID"),&[],max_rows,Duration::from_secs(5))?;
        let mut keys = BTreeSet::new();
        for row in &rows.rows {
            let key = capture::text(row, "native_key");
            if key.is_empty() || !keys.insert(key.to_owned()) {
                return Err(Error::Corrupt);
            }
        }
        let complete = rows.complete && reader.is_current(GENERATION)?;
        let mut observations = projection(&reader, rows.rows, context)?;
        if complete && update_state && reader.generation == self.state.generation {
            for key in self.state.keys.difference(&keys) {
                observations.push(capture::absent("mail.message", key, None, context));
            }
        }
        let material = observations
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.observed_at.clear();
                serde_json::to_string(&row).map_err(Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        let next = State {
            generation: if complete && update_state {
                reader.generation.clone()
            } else {
                self.state.generation.clone()
            },
            keys: if complete && update_state {
                keys
            } else {
                self.state.keys.clone()
            },
            material,
            revision: self.state.revision.checked_add(1).ok_or(Error::Conflict)?,
        };
        let batch = capture::Batch {
            source: "mail".into(),
            generation: reader.generation,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
            scan_started_at: context.observed_at.clone(),
            scope: vec![
                "mail.account".into(),
                "mail.mailbox".into(),
                "mail.message".into(),
            ],
            completion_evidence: if complete { "complete" } else { "partial" },
            coverage: coverage(),
            observations,
            blob_handles: std::collections::BTreeMap::new(),
            live: !self.state.material.is_empty(),
        };
        Ok(Pending {
            batch,
            revision: self.state.revision,
            next,
        })
    }
    pub fn poll(
        &mut self,
        path: &Path,
        context: &Context,
        max_rows: usize,
    ) -> Result<Option<Pending>> {
        let pending = self.snapshot(path, context, max_rows, true)?;
        if self.state.material == pending.next.material {
            self.commit(pending)?;
            return Ok(None);
        }
        Ok(Some(pending))
    }
    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        if self.state.revision != pending.revision {
            return Err(Error::Conflict);
        }
        self.state = pending.next;
        Ok(())
    }
}
