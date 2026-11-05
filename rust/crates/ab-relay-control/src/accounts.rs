use crate::{Error, Result};
use serde::Deserialize;
use std::{future::Future, pin::Pin};

pub type SessionFuture<'a> = Pin<Box<dyn Future<Output = Result<Session>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub account: String,
    pub email: String,
    pub signed_in_at: i64,
}

pub trait Accounts: Send + Sync {
    fn session<'a>(&'a self, cookie: &'a str, source: &'a str) -> SessionFuture<'a>;
}

#[derive(Debug, Clone)]
pub struct BetterAuth {
    pub url: String,
    pub client: reqwest::Client,
}
impl BetterAuth {
    pub fn new(url: &str) -> Result<Self> {
        Ok(Self {
            url: url.trim_end_matches('/').to_owned(),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|_| Error::Unavailable)?,
        })
    }
}
#[derive(Deserialize)]
struct Current {
    user: CurrentUser,
    session: CurrentSession,
}
#[derive(Deserialize)]
struct CurrentUser {
    id: String,
    email: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentSession {
    created_at: String,
}
impl Accounts for BetterAuth {
    fn session<'a>(&'a self, cookie: &'a str, source: &'a str) -> SessionFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .get(format!("{}/api/auth/get-session", self.url))
                .header("cookie", cookie)
                .header("x-forwarded-for", source)
                .send()
                .await
                .map_err(|_| Error::Unavailable)?;
            if !response.status().is_success() {
                return Err(Error::Unauthorized);
            }
            let current: Option<Current> = response.json().await.map_err(|_| Error::Unavailable)?;
            let current = current.ok_or(Error::Unauthorized)?;
            let signed_in_at = current
                .session
                .created_at
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Unavailable)?
                .as_second();
            Ok(Session {
                account: current.user.id,
                email: current.user.email,
                signed_in_at,
            })
        })
    }
}
