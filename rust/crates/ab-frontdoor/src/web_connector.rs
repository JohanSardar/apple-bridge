use crate::web::Web;
use ab_trust::{
    Error, Grant,
    oauth::{self, Client, Code},
};
use serde::Serialize;
use std::time::{Duration, Instant};
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub id: String,
    pub code: String,
    pub client_name: String,
    pub redirect_host: String,
    pub scopes: Vec<Grant>,
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
    web: &Web,
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
        let (agent, _) = web
            .authority
            .create_agent(&client.client_name, level, grants)?;
        code.principal.clone_from(&agent.id);
        match web.authority.issue_oauth_code(code) {
            Ok(raw) => {
                query.insert("code".into(), raw);
            }
            Err(error) => {
                web.authority.update(|d| {
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
impl Web {
    pub(crate) fn connector_begin(
        &self,
        client: Client,
        code: Code,
        state: &str,
        origin: &str,
        scopes: Vec<Grant>,
    ) -> Result<Summary, Error> {
        let mut memory = self.memory.lock().map_err(|_| Error::Unavailable)?;
        memory.connectors.retain(|_, p| p.expires > Instant::now());
        if memory.connectors.len() >= 128 {
            return Err(Error::Unavailable);
        }
        let matching = loop {
            let raw = oauth::token(3);
            let number = raw
                .bytes()
                .fold(0u32, |n, b| (n * 256 + u32::from(b)) % 10000);
            let candidate = format!("{number:04}");
            if !memory
                .connectors
                .values()
                .any(|p| p.summary.code == candidate)
            {
                break candidate;
            }
        };
        let dest = url::Url::parse(&code.redirect_uri).map_err(|_| Error::Invalid)?;
        let summary = Summary {
            id: format!("connector:{}", oauth::token(32)),
            code: matching,
            client_name: client.client_name.clone(),
            redirect_host: dest.authority().into(),
            scopes,
            expires_at: oauth::expiry(300),
        };
        memory.connectors.insert(
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
        Ok(summary)
    }
    pub fn pending_connectors(&self) -> Result<Vec<Summary>, Error> {
        let mut memory = self.memory.lock().map_err(|_| Error::Unavailable)?;
        memory.connectors.retain(|_, p| p.expires > Instant::now());
        Ok(memory
            .connectors
            .values()
            .filter(|p| p.redirect.is_none())
            .map(|p| p.summary.clone())
            .collect())
    }
    pub fn decide_connector(&self, id: &str, matching: &str, approve: bool) -> Result<(), Error> {
        let mut memory = self.memory.lock().map_err(|_| Error::Unavailable)?;
        let p = memory
            .connectors
            .get_mut(id)
            .filter(|p| p.expires > Instant::now() && p.redirect.is_none() && p.attempts < 5)
            .ok_or(Error::Unauthorized)?;
        p.attempts += 1;
        if p.summary.code != matching {
            return Err(Error::Unauthorized);
        }
        p.redirect = Some(decision(
            self,
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
    pub(crate) fn poll_connector(&self, id: &str, origin: &str) -> Result<String, Error> {
        let mut memory = self.memory.lock().map_err(|_| Error::Unavailable)?;
        let p = memory
            .connectors
            .get(id)
            .filter(|p| p.origin == origin && p.expires > Instant::now())
            .ok_or(Error::Unauthorized)?;
        if let Some(redirect) = p.redirect.clone() {
            memory.connectors.remove(id);
            Ok(redirect)
        } else {
            Ok(String::new())
        }
    }
}
