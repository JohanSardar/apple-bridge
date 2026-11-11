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
    pub show_code: bool,
    pub source: String,
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
    pub route: String,
    pub expires: Instant,
    pub redirect: Option<String>,
    pub attempts: u8,
}
#[derive(Debug, Serialize)]
pub(crate) struct Poll {
    pub redirect: String,
    pub show_code: bool,
}
pub const ARMED_FILE: &str = "remote/connect-armed";
pub const ARMED_FOR: Duration = Duration::from_mins(15);
pub(crate) fn source_of(origin: &str, route: &str) -> &'static str {
    if origin.starts_with("http://") {
        "this_mac"
    } else if route == "tailnet" {
        "tailnet"
    } else {
        "public"
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn decision(
    authority: &Authority,
    client: &Client,
    mut code: Code,
    state: &str,
    origin: &str,
    route: &str,
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
        let over_public_host = origin.starts_with("https://");
        let (agent, _) =
            authority.create_oauth_agent(&client.client_name, level, grants, over_public_host)?;
        if over_public_host {
            authority.record_connection(&agent.id, origin, route)?;
        }
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
pub const AGENT_SERVICES: [&str; 12] = [
    "calendar",
    "reminders",
    "notes",
    "contacts",
    "messages",
    "mail",
    "photos",
    "files",
    "safari",
    "usage",
    "location",
    "maps",
];
#[derive(Debug)]
struct AgentAsk {
    summary: Summary,
    secret: String,
    resource: Option<String>,
    expires: Instant,
    outcome: Option<Result<String, ()>>,
}
#[derive(Debug, Serialize)]
pub struct AgentTicket {
    pub id: String,
    pub secret: String,
    pub expires_at: String,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AgentAnswer {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
pub struct Connectors {
    authority: Arc<Authority>,
    pending: Mutex<HashMap<String, Pending>>,
    asks: Mutex<HashMap<String, AgentAsk>>,
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
            asks: Mutex::default(),
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
        route: &str,
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
        let source = source_of(origin, route);
        let summary = Summary {
            id: format!("connector:{}", oauth::token(32)),
            code: matching,
            show_code: false,
            source: source.into(),
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
                route: route.into(),
                expires: Instant::now() + Duration::from_mins(5),
                redirect: None,
                attempts: 0,
            },
        );
        self.reveal(&mut memory);
        let summary = memory
            .get(&summary.id)
            .map_or(summary, |p| p.summary.clone());
        drop(memory);
        if let Some(waiting) = self.waiting.get() {
            waiting();
        }
        Ok(summary)
    }
    fn armed(&self) -> bool {
        std::fs::read_to_string(self.authority.home().join(ARMED_FILE))
            .ok()
            .and_then(|text| text.trim().parse::<i64>().ok())
            .is_some_and(|until| until > jiff::Timestamp::now().as_second())
    }
    fn reveal(&self, memory: &mut HashMap<String, Pending>) {
        let asking = self.asks.lock().map_or(0, |asks| {
            asks.values()
                .filter(|a| a.outcome.is_none() && a.expires > Instant::now())
                .count()
        });
        let waiting = memory.values().filter(|p| p.redirect.is_none()).count() + asking;
        let armed = memory
            .values()
            .any(|p| p.redirect.is_none() && p.summary.source == "public")
            && self.armed();
        for p in memory.values_mut().filter(|p| p.redirect.is_none()) {
            if waiting > 1 || (p.summary.source == "public" && !armed) {
                p.summary.show_code = true;
            }
        }
    }
    pub fn pending(&self) -> Result<Vec<Summary>, Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        memory.retain(|_, p| p.expires > Instant::now());
        self.reveal(&mut memory);
        let mut waiting: Vec<Summary> = memory
            .values()
            .filter(|p| p.redirect.is_none())
            .map(|p| p.summary.clone())
            .collect();
        drop(memory);
        let mut asks = self.asks.lock().map_err(|_| Error::Unavailable)?;
        asks.retain(|_, a| a.expires > Instant::now());
        waiting.extend(
            asks.values()
                .filter(|a| a.outcome.is_none())
                .map(|a| a.summary.clone()),
        );
        Ok(waiting)
    }
    pub fn ask_for_agent(
        &self,
        name: &str,
        source: &str,
        resource: Option<String>,
    ) -> Result<AgentTicket, Error> {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 60
            || name.chars().any(char::is_control)
            || !["this_mac", "tailnet"].contains(&source)
            || (source == "tailnet") != resource.is_some()
        {
            return Err(Error::Invalid);
        }
        if self.waiting.get().is_none() {
            return Err(Error::Unavailable);
        }
        let mut asks = self.asks.lock().map_err(|_| Error::Unavailable)?;
        asks.retain(|_, a| a.expires > Instant::now());
        if asks.values().filter(|a| a.outcome.is_none()).count() >= 4 {
            return Err(Error::Unavailable);
        }
        let scopes = agent_scopes();
        let now = jiff::Timestamp::now();
        let expires_at = jiff::Timestamp::new(now.as_second() + 300, now.subsec_nanosecond())
            .map_err(|_| Error::Unavailable)?
            .to_string();
        let id = format!("agent:{}", oauth::token(32));
        let secret = oauth::token(32);
        asks.insert(
            id.clone(),
            AgentAsk {
                summary: Summary {
                    id: id.clone(),
                    code: oauth::token(8),
                    show_code: false,
                    source: source.into(),
                    client_name: name.into(),
                    redirect_host: String::new(),
                    access: crate::web_pages::access_sentence(&scopes),
                    scopes,
                    expires_at: expires_at.clone(),
                },
                secret: secret.clone(),
                resource,
                expires: Instant::now() + Duration::from_mins(5),
                outcome: None,
            },
        );
        drop(asks);
        if let Some(waiting) = self.waiting.get() {
            waiting();
        }
        Ok(AgentTicket {
            id,
            secret,
            expires_at,
        })
    }
    pub fn agent_answer(&self, id: &str, secret: &str) -> Result<AgentAnswer, Error> {
        let mut asks = self.asks.lock().map_err(|_| Error::Unavailable)?;
        asks.retain(|_, a| a.expires > Instant::now());
        let ask = asks
            .get(id)
            .filter(|a| {
                !secret.is_empty()
                    && a.secret.len() == secret.len()
                    && a.secret
                        .bytes()
                        .zip(secret.bytes())
                        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
                        == 0
            })
            .ok_or(Error::Unauthorized)?;
        let answer = match &ask.outcome {
            None => AgentAnswer {
                state: "waiting",
                token: None,
            },
            Some(Ok(token)) => AgentAnswer {
                state: "allowed",
                token: Some(token.clone()),
            },
            Some(Err(())) => AgentAnswer {
                state: "denied",
                token: None,
            },
        };
        if answer.state != "waiting" {
            asks.remove(id);
        }
        Ok(answer)
    }
    fn decide_ask(&self, id: &str, approve: bool) -> Result<(), Error> {
        let mut asks = self.asks.lock().map_err(|_| Error::Unavailable)?;
        let ask = asks
            .get_mut(id)
            .filter(|a| a.expires > Instant::now() && a.outcome.is_none())
            .ok_or(Error::Unauthorized)?;
        if !approve {
            ask.outcome = Some(Err(()));
            return Ok(());
        }
        let expires = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_hours(24 * 365))
            .ok();
        let (agent, token) = self.authority.create_agent_until(
            &ask.summary.client_name,
            ab_trust::AgentLevel::DEFAULT,
            ask.summary.scopes.clone(),
            expires,
        )?;
        if let Some(resource) = &ask.resource {
            self.authority.bind_to_resource(&agent.id, resource)?;
        }
        ask.outcome = Some(Ok(token));
        Ok(())
    }
    pub fn decide(&self, id: &str, matching: &str, approve: bool) -> Result<(), Error> {
        if id.starts_with("agent:") {
            return self.decide_ask(id, approve);
        }
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
            &p.route,
            approve,
            ab_trust::AgentLevel::DEFAULT_NAME,
            &p.summary.scopes,
        )?);
        Ok(())
    }
    pub(crate) fn poll(&self, id: &str, origin: &str) -> Result<Poll, Error> {
        let mut memory = self.pending.lock().map_err(|_| Error::Unavailable)?;
        memory.retain(|_, p| p.expires > Instant::now());
        self.reveal(&mut memory);
        let p = memory
            .get(id)
            .filter(|p| p.origin == origin)
            .ok_or(Error::Unauthorized)?;
        let show_code = p.summary.show_code;
        if let Some(redirect) = p.redirect.clone() {
            memory.remove(id);
            Ok(Poll {
                redirect,
                show_code,
            })
        } else {
            Ok(Poll {
                redirect: String::new(),
                show_code,
            })
        }
    }
}

fn agent_scopes() -> Vec<Grant> {
    AGENT_SERVICES
        .iter()
        .filter(|service| ab_trust::agent_source(service))
        .filter_map(|service| Grant::parse(&format!("{service}:write")).ok())
        .collect()
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
                "system",
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
                .expect("poll")
                .redirect,
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
            .expect("poll")
            .redirect;
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
    fn an_app_allowed_on_the_mac_acts_without_asking_first() {
        let (_home, queue) = queue();
        let summary = request(&queue, "https://mac.ab.example.com");
        queue
            .decide(&summary.id, &summary.code, true)
            .expect("decide");
        let agents = queue.authority.agents().expect("agents");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].level, "full");
        assert!(agents[0].remote);
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
    fn the_code_shows_only_when_a_request_could_be_confused() {
        let (home, queue) = queue();
        let local = request(&queue, "http://127.0.0.1:7780");
        assert!(!local.show_code, "one request from this Mac needs no code");
        let second = request(&queue, "http://127.0.0.1:7780");
        assert!(second.show_code, "two at once must be told apart");
        assert!(
            queue
                .pending()
                .expect("pending")
                .iter()
                .all(|p| p.show_code)
        );
        assert!(
            queue
                .poll(&local.id, "http://127.0.0.1:7780")
                .expect("poll")
                .show_code
        );

        let (home_two, public) = super::tests::queue();
        assert!(
            request(&public, "https://mac.example.ts.net").show_code,
            "an unexpected request from the internet shows the code"
        );
        drop(home_two);

        let (armed_home, armed) = super::tests::queue();
        std::fs::create_dir_all(armed_home.path().join("remote")).expect("remote");
        std::fs::write(
            armed_home.path().join(ARMED_FILE),
            (jiff::Timestamp::now().as_second() + 600).to_string(),
        )
        .expect("arm");
        assert!(
            !request(&armed, "https://mac.example.ts.net").show_code,
            "a request the owner just started needs no code"
        );
        drop(home);
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
