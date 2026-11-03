use crate::{CommandResponse, Error, frame, raw};
use ab_trust::{ApprovalReauthorizer, Authority};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub(crate) struct Session {
    pub certificate: Vec<u8>,
    pub outgoing: mpsc::Sender<Result<ab_trust::wire::messages::FeedFrame, Error>>,
    pub pending: Mutex<BTreeMap<String, oneshot::Sender<CommandResponse>>>,
}
impl Session {
    pub(crate) fn deliver(&self, frame: &ab_trust::wire::messages::FeedFrame) -> Result<(), Error> {
        if frame.id.is_empty() {
            return Err(Error::Invalid);
        }
        let response = serde_json::from_str(frame.response.as_ref().ok_or(Error::Invalid)?.get())?;
        if let Some(sender) = self
            .pending
            .lock()
            .map_err(|_| Error::Disconnected)?
            .remove(&frame.id)
        {
            let _ = sender.send(response);
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Delegations {
    pub authority: Arc<Authority>,
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
}
impl Delegations {
    pub fn new(authority: Arc<Authority>) -> Self {
        Self {
            authority,
            sessions: Mutex::default(),
        }
    }
    pub(crate) fn register(
        self: &Arc<Self>,
        id: String,
        session: Arc<Session>,
    ) -> Result<Registration, Error> {
        self.sessions
            .lock()
            .map_err(|_| Error::Disconnected)?
            .insert(id.clone(), session.clone());
        Ok(Registration {
            hub: self.clone(),
            id,
            session,
        })
    }
}
#[derive(Debug)]
pub(crate) struct Registration {
    hub: Arc<Delegations>,
    id: String,
    pub session: Arc<Session>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.hub.sessions.lock()
            && sessions
                .get(&self.id)
                .is_some_and(|s| Arc::ptr_eq(s, &self.session))
        {
            sessions.remove(&self.id);
        }
        if let Ok(mut pending) = self.session.pending.lock() {
            pending.clear();
        }
    }
}
struct Pending {
    session: Arc<Session>,
    id: String,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.session.pending.lock() {
            pending.remove(&self.id);
        }
    }
}
impl ApprovalReauthorizer for Delegations {
    fn reauthorize<'a>(
        &'a self,
        principal: &'a str,
        command: &'a str,
        hash: &'a str,
    ) -> ab_trust::Reauthorization<'a> {
        Box::pin(async move {
            let (parent, local) = principal
                .split_once('/')
                .ok_or(ab_trust::Error::Unauthorized)?;
            if !local.starts_with("agent:")
                || local.len() <= 6
                || local.contains('/')
                || self.authority.principal(parent)?.kind != "replica"
            {
                return Err(ab_trust::Error::Unauthorized);
            }
            let unavailable = || ab_trust::Error::Unavailable;
            let session = self
                .sessions
                .lock()
                .map_err(|_| unavailable())?
                .get(parent)
                .cloned()
                .ok_or_else(unavailable)?;
            let challenge = ab_trust::fingerprint(&rand_nonce());
            let id = format!("delegation-{challenge}");
            let (sender, receiver) = oneshot::channel();
            {
                let mut pending = session.pending.lock().map_err(|_| unavailable())?;
                if pending.len() >= 16 {
                    return Err(unavailable());
                }
                pending.insert(id.clone(), sender);
            }
            let _pending = Pending {
                session: session.clone(),
                id: id.clone(),
            };
            let mut request = frame("DelegationRequest");
            request.id = id;
            request.request = Some(raw(&serde_json::json!({"principal":local,"command_id":command,"request_hash":hash,"challenge":challenge})).map_err(|_| unavailable())?);
            let reply = tokio::time::timeout(Duration::from_secs(5), async {
                session
                    .outgoing
                    .send(Ok(request))
                    .await
                    .map_err(|_| unavailable())?;
                receiver.await.map_err(|_| unavailable())
            })
            .await
            .map_err(|_| unavailable())??;
            if let Some(error) = reply.error {
                return Err(if error.code == ab_trust::domain::ErrorCode::Unauthorized {
                    ab_trust::Error::Unauthorized
                } else {
                    unavailable()
                });
            }
            let proof: String =
                serde_json::from_str(reply.result.ok_or(ab_trust::Error::Unauthorized)?.get())
                    .map_err(|_| ab_trust::Error::Unauthorized)?;
            let actor = self.authority.verify_delegation(
                &session.certificate,
                &proof,
                command,
                hash,
                &challenge,
            )?;
            if actor.id != principal
                || actor.kind != "delegated"
                || !["read", "approve", "full"].contains(&actor.level.as_str())
            {
                return Err(ab_trust::Error::Unauthorized);
            }
            Ok(actor)
        })
    }
}
fn rand_nonce() -> [u8; 32] {
    use rand::RngCore;
    let mut nonce = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_trust::Principal;

    #[tokio::test]
    async fn approval_proofs_cannot_be_replayed_for_a_fresh_challenge()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let replica_home = tempfile::tempdir()?;
        let authority = Arc::new(Authority::open(home.path())?);
        let replica = Arc::new(Authority::open(replica_home.path())?);
        let grants = ab_trust::parse_grants("reminders:write")?;
        authority.update(|credentials| {
            credentials.principals.push(Principal {
                id: "replica:test".into(),
                kind: "replica".into(),
                grants: grants.clone(),
                fingerprint: replica.fingerprint.clone(),
                ..Principal::default()
            });
            Ok(())
        })?;
        replica.save_enrollment(serde_json::from_value(serde_json::json!({
            "replica_id":"replica:test", "address":"localhost:7443",
            "fingerprint":authority.fingerprint, "grants":grants,
        }))?)?;
        let actor = Principal {
            id: "agent:test".into(),
            kind: "agent".into(),
            level: "approve".into(),
            grants,
            ..Principal::default()
        };
        let hub = Arc::new(Delegations::new(authority));
        let (outgoing, mut incoming) = mpsc::channel(4);
        let session = Arc::new(Session {
            certificate: replica.certificate.clone(),
            outgoing,
            pending: Mutex::default(),
        });
        let registration = hub.register("replica:test".into(), session.clone())?;
        let responder = tokio::spawn(async move {
            let mut saved = None;
            for _ in 0..2 {
                let request = incoming.recv().await.ok_or("request")??;
                let body: serde_json::Value =
                    serde_json::from_str(request.request.ok_or("body")?.get())?;
                let challenge = body["challenge"].as_str().ok_or("challenge")?;
                let proof = replica.sign_delegation(&actor, "command", "hash", challenge)?;
                let proof = saved.get_or_insert(proof).clone();
                let sender = session
                    .pending
                    .lock()
                    .map_err(|_| "lock")?
                    .remove(&request.id)
                    .ok_or("pending")?;
                sender
                    .send(CommandResponse {
                        result: Some(raw(&proof)?),
                        error: None,
                        lookup_only: false,
                    })
                    .map_err(|_| "send")?;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        });
        let actor = hub
            .reauthorize("replica:test/agent:test", "command", "hash")
            .await?;
        assert_eq!(actor.level, "approve");
        assert!(matches!(
            hub.reauthorize("replica:test/agent:test", "command", "hash")
                .await,
            Err(ab_trust::Error::Unauthorized)
        ));
        responder.await?.map_err(|e| e.to_string())?;
        drop(registration);
        assert!(matches!(
            hub.reauthorize("replica:test/agent:test", "command", "hash")
                .await,
            Err(ab_trust::Error::Unavailable)
        ));
        Ok(())
    }
}
