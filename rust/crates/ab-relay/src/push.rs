use crate::{Error, Result, http::Api};
use ab_wire::crypto::Packet;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use rustls::pki_types::pem::PemObject;
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

pub trait PushProvider: std::fmt::Debug + Send + Sync {
    fn send<'a>(
        &'a self,
        token: &'a str,
        packet: &'a Packet,
        expires: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;
}
#[derive(Debug)]
pub struct Apns {
    key: ring::signature::EcdsaKeyPair,
    client: reqwest::Client,
    sandbox: bool,
}
impl Apns {
    pub fn load(path: &std::path::Path, sandbox: bool) -> Result<Self> {
        let pem = std::fs::read(path)?;
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from_pem_slice(&pem)
            .map_err(|_| Error::Configuration)?;
        let key = ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            key.secret_pkcs8_der(),
            &ring::rand::SystemRandom::new(),
        )
        .map_err(|_| Error::Configuration)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            key,
            client,
            sandbox,
        })
    }
    async fn publish(&self, device: &str, packet: &Packet, expires: i64) -> Result<()> {
        if !(32..=512).contains(&device.len())
            || !device
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Denied);
        }
        let live = packet.live_activity.as_ref();
        let body = match live {
            Some(live) => json!({"aps":live}),
            None => json!({"aps":{"content-available":1},"bridge":packet}),
        };
        let body = serde_json::to_vec(&body)?;
        if body.len() > 4096 {
            return Err(Error::Denied);
        }
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","kid":"L2854KRF5Z"}"#);
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(
            &json!({"iat":jiff::Timestamp::now().as_second(),"iss":"AX3C2B78YD"}),
        )?);
        let encoded = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&ring::rand::SystemRandom::new(), encoded.as_bytes())
            .map_err(|_| Error::Denied)?;
        let endpoint = if self.sandbox {
            "https://api.sandbox.push.apple.com"
        } else {
            "https://api.push.apple.com"
        };
        let response = self
            .client
            .post(format!("{endpoint}/3/device/{device}"))
            .header(
                "authorization",
                format!(
                    "bearer {encoded}.{}",
                    URL_SAFE_NO_PAD.encode(signature.as_ref())
                ),
            )
            .header(
                "apns-topic",
                if live.is_some() {
                    "com.johansardar.applebridge.ios.push-type.liveactivity"
                } else {
                    "com.johansardar.applebridge.ios"
                },
            )
            .header(
                "apns-push-type",
                if live.is_some() {
                    "liveactivity"
                } else {
                    "background"
                },
            )
            .header("apns-priority", if live.is_some() { "10" } else { "5" })
            .header("apns-expiration", expires.to_string())
            .body(body)
            .send()
            .await
            .map_err(|_| Error::Denied)?;
        if response.status() != 200 {
            return Err(Error::Denied);
        }
        Ok(())
    }
}
impl PushProvider for Apns {
    fn send<'a>(
        &'a self,
        token: &'a str,
        packet: &'a Packet,
        expires: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.publish(token, packet, expires))
    }
}
#[derive(Debug, Default)]
pub struct Push {
    pub provider: Option<Arc<dyn PushProvider>>,
    rates: Mutex<HashMap<String, (tokio::time::Instant, u64)>>,
}
fn empty_packet() -> Packet {
    Packet {
        live_activity: None,
        ephemeral_key: String::new(),
        r#box: String::new(),
        signature: String::new(),
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    token: String,
    #[serde(default = "empty_packet")]
    packet: Packet,
    #[serde(default)]
    expires: i64,
}
pub async fn handle(api: &Api, request: Request, token: &str) -> Response {
    if !api
        .server
        .store
        .lock()
        .await
        .authenticated(&request.hostname, token)
    {
        return crate::http::error(403, "denied");
    }
    let Some(provider) = &api.push.provider else {
        return crate::http::error(503, "push unavailable");
    };
    let now = jiff::Timestamp::now().as_second();
    if request.expires <= now
        || request.expires > now + 600
        || STANDARD
            .decode(&request.packet.ephemeral_key)
            .map_or(0, |b| b.len())
            != 32
        || STANDARD
            .decode(&request.packet.signature)
            .map_or(0, |b| b.len())
            != 64
        || !STANDARD
            .decode(&request.packet.r#box)
            .is_ok_and(|b| (28..=2800).contains(&b.len()))
    {
        return crate::http::error(400, "invalid");
    }
    {
        let mut rates = api.push.rates.lock().await;
        let entry = rates
            .entry(request.hostname)
            .or_insert((tokio::time::Instant::now(), 0));
        if entry.0.elapsed() > Duration::from_secs(60) {
            *entry = (tokio::time::Instant::now(), 0);
        }
        entry.1 += 1;
        if entry.1 > 60 {
            return crate::http::error(429, "rate limited");
        }
    }
    match tokio::time::timeout(
        Duration::from_secs(10),
        provider.send(&request.token, &request.packet, request.expires),
    )
    .await
    {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        _ => crate::http::error(502, "push failed"),
    }
}
