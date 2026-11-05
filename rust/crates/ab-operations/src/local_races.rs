use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Debug, Default)]
struct RaceState;
impl GatewayState for RaceState {
    fn admission_era(&self) -> Result<String, Error> {
        Ok("era".into())
    }
    fn command(&self, _: &str) -> Result<CommandRecord, Error> {
        Err(Code::NotFound.into())
    }
    fn commands(&self) -> Result<Vec<CommandRecord>, Error> {
        Ok(Vec::new())
    }
    fn transition(&self, _: Option<State>, record: CommandRecord) -> Result<CommandRecord, Error> {
        Ok(record)
    }
    fn update_parts(&self, record: CommandRecord) -> Result<CommandRecord, Error> {
        Ok(record)
    }
    fn head(&self) -> Result<Cursor, Error> {
        Ok(Cursor::default())
    }
    fn observed_record_id(&self, _: &str, _: &str, _: &Cursor) -> Result<String, Error> {
        Err(Code::NotFound.into())
    }
}
#[derive(Debug)]
struct RaceIdentity;
impl Identity for RaceIdentity {
    fn authenticate(&self, _: &str) -> Result<Principal, Error> {
        Err(Code::Unsupported.into())
    }
    fn current(&self, p: &Principal) -> Result<Principal, Error> {
        Ok(p.clone())
    }
    fn sign(&self, _: &Principal, _: &str, _: &str) -> Result<String, Error> {
        Err(Code::Unsupported.into())
    }
}
#[derive(Debug)]
struct RaceCodec;
impl Codec for RaceCodec {
    fn json(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(bytes.to_vec())
    }
    fn command_hash(&self, _: &Command) -> Result<String, Error> {
        Ok("hash".into())
    }
    fn fingerprint(&self, _: &[u8]) -> String {
        "hash".into()
    }
}
#[derive(Debug, Default)]
struct RacePeer(AtomicUsize);
impl ApplePeer for RacePeer {
    fn available(&self) -> bool {
        true
    }
    fn exchange(&self, f: IpcFrame) -> crate::Task<'_, IpcFrame> {
        Box::pin(async move {
            Ok(if f.t == "Prepare" {
                IpcFrame {
                    t: "Prepared".into(),
                    ticket: "ticket".into(),
                    ..Default::default()
                }
            } else {
                self.0.fetch_add(1, Ordering::SeqCst);
                IpcFrame {
                    t: "Outcome".into(),
                    disposition: "locally_observed".into(),
                    ..Default::default()
                }
            })
        })
    }
    fn captured(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Cursor, Error> {
        Err(Code::NotFound.into())
    }
    fn capture_revision(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
}
#[derive(Debug, Default)]
struct RaceActions {
    version: AtomicUsize,
    calls: AtomicUsize,
    selected: tokio::sync::Notify,
}
impl ActionResolver for RaceActions {
    fn serialization_key(&self, _: &Principal, c: &Command) -> Result<String, Error> {
        let key = crate::command_contract::serialization_key(c)?;
        self.selected.notify_one();
        Ok(key)
    }
    fn resolve(&self, _: &Principal, c: &Command) -> Result<PreparedAction, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.version.load(Ordering::SeqCst) != 2 {
            return Err(Code::Conflict.into());
        }
        Ok(PreparedAction {
            action: raw(&json!({"op":c.op,"args":{"id":"rml_future","title":"Synthetic"}}))?,
            expected_state: None,
            collection: "reminders.list".into(),
            serialization_key: "rml_future".into(),
        })
    }
    fn evidence(&self, _: &Command, _: &str, _: &IpcFrame) -> Result<OutcomeEvidence, Error> {
        Ok(OutcomeEvidence::Standalone)
    }
}
#[tokio::test]
async fn queued_version_is_checked_after_the_preceding_object_lock()
-> Result<(), Box<dyn std::error::Error>> {
    let actions = Arc::new(RaceActions::default());
    let peer = Arc::new(RacePeer::default());
    let gateway = Arc::new(LocalGateway::new(
        Arc::new(RaceState),
        Arc::new(RaceIdentity),
        Arc::new(RaceCodec),
        peer.clone(),
        actions.clone(),
    ));
    let object = Arc::new(AsyncMutex::new(()));
    gateway
        .locks
        .lock()
        .map_err(|_| "locks")?
        .insert("object:rml_future".into(), object.clone());
    let preceding = object.lock().await;
    let record: CommandRecord = serde_json::from_value(
        json!({"command":{"command_id":"cmd_race","op":"reminders.rename_list","args":{"id":"rml_future","title":"Synthetic"},"expected_version":2},"principal":"device:race","source":"reminders","request_hash":"hash","state":"admitted","admission_era":"era","created_at":"2026-01-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z","commit_seq":0,"cursor":{"epoch":"","sequence":0,"subscription_revision":0},"history":[]}),
    )?;
    let p = Principal {
        id: "device:race".into(),
        kind: "device".into(),
        grants: vec!["admin".into()],
        level: String::new(),
        resource: String::new(),
        remote: false,
        expires_at: None,
    };
    let task = tokio::spawn(async move {
        gateway
            .execute(
                record,
                p,
                tokio::time::Instant::now() + Duration::from_secs(1),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), actions.selected.notified()).await?;
    assert_eq!(actions.calls.load(Ordering::SeqCst), 0);
    actions.version.store(2, Ordering::SeqCst);
    drop(preceding);
    let record = tokio::time::timeout(Duration::from_secs(1), task).await???;
    assert_eq!(record.state, State::Observed);
    assert_eq!(actions.calls.load(Ordering::SeqCst), 1);
    assert_eq!(peer.0.load(Ordering::SeqCst), 1);
    Ok(())
}
