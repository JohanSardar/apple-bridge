use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{future::Future, net::SocketAddr, pin::Pin};

pub type RelayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Admitted {
    pub hostname: String,
    pub credential: String,
}
impl std::fmt::Debug for Admitted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admitted")
            .field("hostname", &self.hostname)
            .finish_non_exhaustive()
    }
}

pub trait RelayAdmin: Send + Sync {
    fn admit<'a>(&'a self, public_key: &'a str) -> RelayFuture<'a, Admitted>;
    fn policy<'a>(
        &'a self,
        hostname: &'a str,
        disabled: bool,
        revoked: bool,
        version: i64,
    ) -> RelayFuture<'a, ()>;
}

pub struct RelayApi {
    url: String,
    token: String,
    client: reqwest::Client,
}
impl std::fmt::Debug for RelayApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayApi")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}
impl RelayApi {
    pub fn new(url: &str, token: String, connect: Option<SocketAddr>) -> Result<Self> {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .ok_or(Error::Invalid)?;
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10));
        if let Some(address) = connect {
            builder = builder.resolve(&host, address);
        }
        Ok(Self {
            url: url.trim_end_matches('/').to_owned(),
            token,
            client: builder.build().map_err(|_| Error::Unavailable)?,
        })
    }
}
impl RelayAdmin for RelayApi {
    fn admit<'a>(&'a self, public_key: &'a str) -> RelayFuture<'a, Admitted> {
        Box::pin(async move {
            let response = self
                .client
                .post(format!("{}/admit", self.url))
                .bearer_auth(&self.token)
                .json(&json!({ "public_key": public_key }))
                .send()
                .await
                .map_err(|_| Error::Unavailable)?;
            if !response.status().is_success() {
                return Err(Error::Unavailable);
            }
            response.json().await.map_err(|_| Error::Unavailable)
        })
    }
    fn policy<'a>(
        &'a self,
        hostname: &'a str,
        disabled: bool,
        revoked: bool,
        version: i64,
    ) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            let response = self
                .client
                .post(format!("{}/policy", self.url))
                .bearer_auth(&self.token)
                .json(&json!({"hostname": hostname, "disabled": disabled, "revoked": revoked, "version": version}))
                .send()
                .await
                .map_err(|_| Error::Unavailable)?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(Error::Unavailable)
            }
        })
    }
}
