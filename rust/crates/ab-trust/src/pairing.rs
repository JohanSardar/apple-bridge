use crate::{Authority, Error, Grant, Principal};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::Mac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Enrollment {
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub pairing_id: String,
    #[serde(default)]
    pub dial: String,
    #[serde(default)]
    pub ceiling: Vec<Grant>,
    pub replica_id: String,
    pub address: String,
    pub fingerprint: String,
    pub grants: Vec<Grant>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairCode {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub dial: String,
    pub id: String,
    pub address: String,
    pub fingerprint: String,
    pub secret: String,
    pub expires_at: String,
}
impl PairCode {
    pub fn decode(code: &str) -> Result<Self, Error> {
        let raw = data_encoding::BASE32_NOPAD
            .decode(code.strip_prefix("ab1-").ok_or(Error::Invalid)?.as_bytes())
            .map_err(|_| Error::Invalid)?;
        let c: Self = serde_json::from_slice(&raw)?;
        if c.id.is_empty()
            || c.address.is_empty()
            || c.fingerprint.len() != 64
            || !c.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || STANDARD
                .decode(&c.secret)
                .map_err(|_| Error::Invalid)?
                .len()
                != 20
            || c.expires_at
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?
                < jiff::Timestamp::now()
            || !c.purpose.is_empty()
        {
            return Err(Error::Invalid);
        }
        Ok(c)
    }
    pub fn proof(&self, cert: &[u8]) -> Result<String, Error> {
        Ok(STANDARD.encode(pair_proof(
            &STANDARD.decode(&self.secret).map_err(|_| Error::Invalid)?,
            cert,
        )?))
    }
}
pub fn pair_proof(secret: &[u8], cert: &[u8]) -> Result<Vec<u8>, Error> {
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&Sha256::digest(secret))
        .map_err(|_| Error::Invalid)?;
    mac.update(b"apple-bridge-pair/1\0");
    mac.update(cert);
    Ok(mac.finalize().into_bytes().to_vec())
}
impl Authority {
    pub async fn join(&self, code: &str) -> Result<Enrollment, Error> {
        let c = PairCode::decode(code)?;
        if c.remote || !c.dial.is_empty() {
            return Err(Error::Invalid);
        }
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(self.client_tls(&c.fingerprint)?)
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let p: Principal = client
            .post(format!("https://{}/v1/pair", c.address))
            .json(&serde_json::json!({"id":c.id,"proof":c.proof(&self.certificate)?}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if p.kind != "replica" || p.id.is_empty() || p.fingerprint != self.fingerprint {
            return Err(Error::Unauthorized);
        }
        let e = Enrollment {
            remote: false,
            pairing_id: c.id,
            dial: c.dial,
            replica_id: p.id,
            address: c.address,
            fingerprint: c.fingerprint,
            ceiling: p.grants.clone(),
            grants: p.grants,
        };
        self.save_enrollment(e.clone())?;
        Ok(e)
    }
}

impl Authority {
    #[allow(clippy::too_many_lines)]
    pub async fn join_listen(&self, code: &str, address: &str) -> Result<Enrollment, Error> {
        use http_body_util::BodyExt;
        use std::sync::Arc;
        let c = Arc::new(PairCode::decode(code)?);
        if self.enrollment()?.is_some_and(|e| e.pairing_id == c.id) {
            return Err(Error::Unauthorized);
        }
        if c.remote || c.dial.is_empty() {
            return Err(Error::Invalid);
        }
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(self.server_tls(&c.fingerprint)?));
        let listener = tokio::net::TcpListener::bind(address).await?;
        let remaining = c
            .expires_at
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid)?
            .as_second()
            - jiff::Timestamp::now().as_second();
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(u64::try_from(remaining).map_err(|_| Error::Invalid)?);
        loop {
            let (socket, _) = tokio::time::timeout_at(deadline, listener.accept())
                .await
                .map_err(|_| Error::Unauthorized)??;
            let stream =
                tokio::time::timeout(std::time::Duration::from_secs(5), acceptor.accept(socket))
                    .await
                    .map_err(|_| Error::Unauthorized)?
                    .map_err(Error::Io)?;
            let completed = Arc::new(std::sync::Mutex::new(None));
            let result = completed.clone();
            let code = c.clone();
            let handler = hyper::service::service_fn(
                |request: hyper::Request<hyper::body::Incoming>| {
                    let code = code.clone();
                    let result = result.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        let post = request.method() == hyper::Method::POST;
                        let bytes = http_body_util::Limited::new(request.into_body(), 16384)
                            .collect()
                            .await;
                        let response = (|| -> Result<Vec<u8>, Error> {
                            if !post {
                                return Err(Error::Unauthorized);
                            }
                            let body = bytes.map_err(|_| Error::Invalid)?.to_bytes();
                            if path == "/v1/pair/proof" {
                                let req: serde_json::Value = serde_json::from_slice(&body)?;
                                if req["id"].as_str() != Some(&code.id) {
                                    return Err(Error::Unauthorized);
                                }
                                return Ok(serde_json::to_vec(
                                    &serde_json::json!({"id":code.id,"proof":code.proof(&self.certificate)?}),
                                )?);
                            }
                            if path != "/v1/pair/finish" {
                                return Err(Error::Unauthorized);
                            }
                            let p: Principal = serde_json::from_slice(&body)?;
                            if p.kind != "replica"
                                || p.id.is_empty()
                                || p.fingerprint != self.fingerprint
                                || p.dial != code.dial
                            {
                                return Err(Error::Unauthorized);
                            }
                            let e = Enrollment {
                                remote: false,
                                pairing_id: code.id.clone(),
                                dial: code.dial.clone(),
                                replica_id: p.id,
                                address: code.address.clone(),
                                fingerprint: code.fingerprint.clone(),
                                ceiling: p.grants.clone(),
                                grants: p.grants,
                            };
                            self.save_enrollment(e.clone())?;
                            *result.lock().map_err(|_| Error::Unavailable)? = Some(e);
                            Ok(b"{}".to_vec())
                        })();
                        let (status, body) = match response {
                            Ok(b) => (200, b),
                            Err(_) => (401, b"unauthorized".to_vec()),
                        };
                        let mut response = hyper::Response::new(http_body_util::Full::new(
                            hyper::body::Bytes::from(body),
                        ));
                        *response.status_mut() =
                            hyper::StatusCode::from_u16(status).map_err(|_| Error::Invalid)?;
                        response.headers_mut().insert(
                            "connection",
                            hyper::header::HeaderValue::from_static("close"),
                        );
                        Ok::<_, Error>(response)
                    }
                },
            );
            let serve = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), handler);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), serve).await;
            if let Some(e) = completed.lock().map_err(|_| Error::Unavailable)?.take() {
                return Ok(e);
            }
        }
    }
}
