use crate::Error;
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentActivity {
    #[serde(skip)]
    pub sequence: u64,
    pub id: String,
    pub principal: String,
    pub operation: String,
    pub kind: String,
    pub state: String,
    pub at: ab_wire::gojson::GoTime,
    pub count: u64,
    pub argument_bytes: u64,
    pub argument_hash: String,
}
#[derive(Debug)]
pub struct ActivityLog(Connection);
impl ActivityLog {
    pub fn open(home: impl AsRef<Path>) -> Result<Self, Error> {
        let connection = Connection::open_with_flags(
            home.as_ref().join("state.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self(connection))
    }
    pub fn record(&mut self, activity: &AgentActivity) -> Result<(), Error> {
        let at = activity
            .at
            .as_str()
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid)?
            .as_second();
        let sequence = i64::try_from(activity.sequence).map_err(|_| Error::Invalid)?;
        let bytes = i64::try_from(activity.argument_bytes).map_err(|_| Error::Invalid)?;
        let transaction = self.0.transaction()?;
        transaction.execute(
            "DELETE FROM agent_activity WHERE at < ?",
            [jiff::Timestamp::now().as_second() - 90 * 86400],
        )?;
        transaction.execute("INSERT INTO agent_activity(id,principal,operation,kind,state,at,count,argument_bytes,argument_hash,sequence) VALUES(?,?,?,?,?,?,1,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,count=agent_activity.count+?,argument_bytes=excluded.argument_bytes,argument_hash=excluded.argument_hash,sequence=excluded.sequence WHERE agent_activity.kind='read' OR excluded.sequence>=agent_activity.sequence", params![activity.id, activity.principal, activity.operation, activity.kind, activity.state, at, bytes, activity.argument_hash, sequence, i64::from(activity.kind == "read")])?;
        transaction.commit()?;
        Ok(())
    }
    pub fn list(&self, principal: &str, operation: &str) -> Result<Vec<AgentActivity>, Error> {
        self.0.execute(
            "DELETE FROM agent_activity WHERE at < ?",
            [jiff::Timestamp::now().as_second() - 90 * 86400],
        )?;
        let mut statement = self.0.prepare("SELECT id,principal,operation,kind,state,at,count,argument_bytes,argument_hash FROM agent_activity WHERE (?='' OR principal=?) AND (?='' OR operation=?) ORDER BY at DESC,id DESC LIMIT 1000")?;
        let rows =
            statement.query_map(params![principal, principal, operation, operation], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?;
        let mut activities = Vec::new();
        for row in rows {
            let (id, principal, operation, kind, state, at, count, argument_bytes, argument_hash) =
                row?;
            let timestamp = jiff::Timestamp::from_second(at).map_err(|_| Error::Invalid)?;
            activities.push(AgentActivity {
                id,
                principal,
                operation,
                kind,
                state,
                at: ab_wire::gojson::GoTime::parse(&timestamp.to_string())
                    .map_err(|_| Error::Invalid)?,
                count: u64::try_from(count).map_err(|_| Error::Invalid)?,
                argument_bytes: u64::try_from(argument_bytes).map_err(|_| Error::Invalid)?,
                argument_hash,
                sequence: 0,
            });
        }
        Ok(activities)
    }
}
