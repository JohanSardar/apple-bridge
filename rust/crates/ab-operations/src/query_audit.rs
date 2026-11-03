use crate::{Code, Error, Principal};
use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct QueryAudit {
    pub operation: String,
    pub argument_bytes: usize,
    pub result_bytes: usize,
    pub blob_bytes: i64,
}
pub trait QueryAuditSink: Debug + Send + Sync {
    fn record(
        &self,
        p: &Principal,
        id: &str,
        audit: &QueryAudit,
        completed: bool,
    ) -> Result<(), Error>;
}
#[derive(Debug)]
pub struct QueryActivity(pub Mutex<ab_trust::activity::ActivityLog>);
impl QueryActivity {
    pub fn open(home: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        Ok(Self(Mutex::new(
            ab_trust::activity::ActivityLog::open(home).map_err(|_| Code::Unavailable)?,
        )))
    }
}
impl QueryAuditSink for QueryActivity {
    fn record(
        &self,
        p: &Principal,
        id: &str,
        audit: &QueryAudit,
        completed: bool,
    ) -> Result<(), Error> {
        if !completed || !p.is_agent() {
            return Ok(());
        }
        let activity = ab_trust::activity::AgentActivity {
            sequence: 0,
            id: format!("{}:{id}", p.id),
            principal: p.id.clone(),
            operation: audit.operation.clone(),
            kind: "query".into(),
            state: "finished".into(),
            at: ab_trust::wire::gojson::GoTime::parse(&jiff::Timestamp::now().to_string())
                .map_err(|_| Code::Corrupt)?,
            count: 1,
            argument_bytes: u64::try_from(audit.argument_bytes).map_err(|_| Code::Corrupt)?,
            argument_hash: String::new(),
        };
        let call = || {
            self.0
                .lock()
                .map_err(|_| Code::Unavailable)?
                .record(&activity)
                .map_err(|_| Code::Unavailable.into())
        };
        if tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
        {
            tokio::task::block_in_place(call)
        } else {
            call()
        }
    }
}
#[derive(Debug)]
pub(crate) struct AuditGuard {
    sink: Option<Arc<dyn QueryAuditSink>>,
    principal: Principal,
    id: String,
    pub stats: QueryAudit,
    done: bool,
    deadline: tokio::time::Instant,
}
impl AuditGuard {
    pub fn new(
        sink: Option<Arc<dyn QueryAuditSink>>,
        p: &Principal,
        id: &str,
        operation: &str,
        argument_bytes: usize,
        deadline: tokio::time::Instant,
    ) -> Self {
        Self {
            sink,
            principal: p.clone(),
            id: id.into(),
            stats: QueryAudit {
                operation: operation.into(),
                argument_bytes,
                ..Default::default()
            },
            done: false,
            deadline,
        }
    }
    pub fn finish(&mut self) -> Result<(), Error> {
        self.done = true;
        if let Some(sink) = &self.sink {
            sink.record(
                &self.principal,
                &self.id,
                &self.stats,
                tokio::time::Instant::now() < self.deadline,
            )?;
        }
        Ok(())
    }
}
impl Drop for AuditGuard {
    fn drop(&mut self) {
        if !self.done
            && let Some(sink) = &self.sink
        {
            let _ = sink.record(&self.principal, &self.id, &self.stats, false);
        }
    }
}
