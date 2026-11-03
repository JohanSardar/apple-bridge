#![forbid(unsafe_code)]
pub use ab_wire::crypto::Packet;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("iPhone push unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub hostname: String,
    pub token: String,
    pub packet: Packet,
    pub expires: i64,
}

#[derive(Clone)]
pub struct PushClient {
    endpoint: reqwest::Url,
    hostname: String,
    credential: String,
    client: reqwest::Client,
}
impl PushClient {
    pub fn new(endpoint: &str, hostname: String, credential: String) -> Result<Self, Error> {
        let mut endpoint = reqwest::Url::parse(endpoint).map_err(|_| Error::Unavailable)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || hostname.is_empty()
            || credential.is_empty()
        {
            return Err(Error::Unavailable);
        }
        endpoint.set_path(&format!(
            "{}/iphone/push",
            endpoint.path().trim_end_matches('/')
        ));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            endpoint,
            hostname,
            credential,
            client,
        })
    }
    pub async fn push(&self, token: String, packet: Packet, expires: i64) -> Result<(), Error> {
        let request = PushRequest {
            hostname: self.hostname.clone(),
            token,
            packet,
            expires,
        };
        let raw = ab_wire::gojson::legacy_json(&request, ab_wire::gojson::EscapeMode::Html)
            .map_err(|_| Error::Unavailable)?;
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.credential)
            .header("Content-Type", "application/json")
            .body(raw)
            .send()
            .await?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}

impl std::fmt::Debug for PushClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushClient")
            .field("endpoint", &self.endpoint)
            .field("hostname", &self.hostname)
            .finish_non_exhaustive()
    }
}
