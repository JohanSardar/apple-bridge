use ab_trust::{
    Authority, Error, Grant,
    oauth::{self, Client, Code},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub id: String,
    pub code: String,
    pub client_name: String,
    pub redirect_host: String,
    pub scopes: Vec<Grant>,
    pub access: String,
    pub expires_at: String,
}
#[derive(Debug)]
pub(crate) struct Pending {
    pub summary: Summary,
    pub client: Client,
    pub oauth: Code,
    pub state: String,
    pub origin: String,
    pub expires: Instant,
    pub redirect: Option<String>,
    pub attempts: u8,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn decision(
    authority: &Authority,
    client: &Client,
    mut code: Code,
    state: &str,
    origin: &str,
    approve: bool,
    level: &str,
    grants: &[Grant],
) -> Result<String, Error> {
    let mut dest = url::Url::parse(&code.redirect_uri).map_err(|_| Error::Invalid)?;
    let mut query: std::collections::BTreeMap<String, String> = dest
        .query_pairs()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    query.insert("state".into(), state.into());
    query.insert("iss".into(), origin.into());
    if approve {
        let (agent, _) = authority.create_oauth_agent(&client.client_name, level, grants)?;
        code.principal.clone_from(&agent.id);
        match authority.issue_oauth_code(code) {
            Ok(raw) => {
                query.insert("code".into(), raw);
            }
            Err(error) => {
                authority.update(|d| {
                    if let Some(p) = d.principals.iter_mut().find(|p| p.id == agent.id) {
                        p.token_hash.clear();
                        p.revision += 1;
                        p.extra.insert(
                            "revoked_at".into(),
                            jiff::Timestamp::now().to_string().into(),
                        );
                    }
                    Ok(())
                })?;
                return Err(error);
            }
        }
    } else {
        query.insert("error".into(), "access_denied".into());
    }
    dest.set_query(Some(
        &url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&query)
            .finish(),
    ));
    Ok(dest.into())
}
pub struct Connectors {
    authority: Arc<Authority>,
    pending: Mutex<HashMap<String, Pending>>,
    waiting: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}
impl std::fmt::Debug for Connectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connectors").finish_non_exhaustive()
    }
}
impl Connectors {
    pub fn new(authority: Arc<Authority>) -> Arc<Self> {
        Arc::new(Self {
            authority,
            pending: Mutex::default(),
            waiting: OnceLock::new(),
        })
    }
    pub fn on_waiting(&self, notify: Arc<dyn Fn() + Send + Sync>) {
        let _ = self.waiting.set(notify);
    }
    pub(crate) fn begin(
        &self,
        client: Client,
        code: Code,
        state: &str,
        origin: &str,
        scopes: Vec<Grant>,
    ) -> Result<Summary, Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        memory.retain(|_, p| p.expires > Instant::now());
        if memory.len() >= 128 {
            return Err(Error::Unavailable);
        }
        let matching = loop {
            let raw = oauth::token(3);
            let number = raw
                .bytes()
                .fold(0u32, |n, b| (n * 256 + u32::from(b)) % 10000);
            let candidate = format!("{number:04}");
            if !memory.values().any(|p| p.summary.code == candidate) {
                break candidate;
            }
        };
        let dest = url::Url::parse(&code.redirect_uri).map_err(|_| Error::Invalid)?;
        let summary = Summary {
            id: format!("connector:{}", oauth::token(32)),
            code: matching,
            client_name: client.client_name.clone(),
            redirect_host: dest.authority().into(),
            access: crate::web_pages::access_sentence(&scopes),
            scopes,
            expires_at: {
                let now = jiff::Timestamp::now();
                jiff::Timestamp::new(now.as_second() + 300, now.subsec_nanosecond())
                    .map_err(|_| Error::Unavailable)?
                    .to_string()
            },
        };
        memory.insert(
            summary.id.clone(),
            Pending {
                summary: summary.clone(),
                client,
                oauth: code,
                state: state.into(),
                origin: origin.into(),
                expires: Instant::now() + Duration::from_mins(5),
                redirect: None,
                attempts: 0,
            },
        );
        drop(memory);
        if let Some(waiting) = self.waiting.get() {
            waiting();
        }
        Ok(summary)
    }
    pub fn pending(&self) -> Result<Vec<Summary>, Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        memory.retain(|_, p| p.expires > Instant::now());
        Ok(memory
            .values()
            .filter(|p| p.redirect.is_none())
            .map(|p| p.summary.clone())
            .collect())
    }
    pub fn decide(&self, id: &str, matching: &str, approve: bool) -> Result<(), Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        let p = memory
            .get_mut(id)
            .filter(|p| p.expires > Instant::now() && p.redirect.is_none() && p.attempts < 5)
            .ok_or(Error::Unauthorized)?;
        p.attempts += 1;
        if p.summary.code != matching {
            return Err(Error::Unauthorized);
        }
        p.redirect = Some(decision(
            &self.authority,
            &p.client,
            p.oauth.clone(),
            &p.state,
            &p.origin,
            approve,
            "approve",
            &p.summary.scopes,
        )?);
        Ok(())
    }
    pub(crate) fn poll(&self, id: &str, origin: &str) -> Result<String, Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        let p = memory
            .get(id)
            .filter(|p| p.origin == origin && p.expires > Instant::now())
            .ok_or(Error::Unauthorized)?;
        if let Some(redirect) = p.redirect.clone() {
            memory.remove(id);
            Ok(redirect)
        } else {
            Ok(String::new())
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn queue() -> (tempfile::TempDir, Arc<Connectors>) {
        let dir = tempfile::tempdir().expect("temp home");
        let authority = Arc::new(Authority::open(dir.path()).expect("authority"));
        (dir, Connectors::new(authority))
    }

    fn client() -> Client {
        Client {
            client_id: "https://claude.ai/oauth/mcp-oauth-client-metadata".into(),
            client_name: "Claude".into(),
            redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
            ..Client::default()
        }
    }

    fn request(queue: &Connectors, origin: &str) -> Summary {
        queue
            .begin(
                client(),
                Code {
                    client_id: client().client_id,
                    redirect_uri: "https://claude.ai/api/mcp/auth_callback".into(),
                    challenge: "x".into(),
                    resource: format!("{origin}/mcp"),
                    ..Code::default()
                },
                "state",
                origin,
                vec![Grant::parse("calendar:read").expect("grant")],
            )
            .expect("begin")
    }

    #[test]
    fn a_request_on_any_host_reaches_the_owner_and_only_its_origin_gets_the_result() {
        let (_home, queue) = queue();
        let summary = request(&queue, "https://mac.ab.example.com");
        assert_eq!(queue.pending().expect("pending").len(), 1);
        assert_eq!(
            queue
                .poll(&summary.id, "https://mac.ab.example.com")
                .expect("poll"),
            ""
        );
        queue
            .decide(&summary.id, &summary.code, true)
            .expect("decide");
        assert!(
            queue
                .poll(&summary.id, "https://other.example.com")
                .is_err()
        );
        let redirect = queue
            .poll(&summary.id, "https://mac.ab.example.com")
            .expect("poll");
        assert!(redirect.starts_with("https://claude.ai/api/mcp/auth_callback?"));
        assert!(redirect.contains("iss=https%3A%2F%2Fmac.ab.example.com"));
        assert!(redirect.contains("code="));
        assert!(
            queue
                .poll(&summary.id, "https://mac.ab.example.com")
                .is_err(),
            "single use"
        );
    }

    #[test]
    fn a_wrong_code_is_refused_and_attempts_are_limited() {
        let (_home, queue) = queue();
        let summary = request(&queue, "https://mac.ab.example.com");
        let wrong = if summary.code == "0000" {
            "0001"
        } else {
            "0000"
        };
        for _ in 0..5 {
            assert!(queue.decide(&summary.id, wrong, true).is_err());
        }
        assert!(
            queue.decide(&summary.id, &summary.code, true).is_err(),
            "locked out"
        );
    }

    #[test]
    fn waiting_is_announced() {
        let (_home, queue) = queue();
        let told = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = told.clone();
        queue.on_waiting(Arc::new(move || {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        request(&queue, "https://mac.ab.example.com");
        assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
