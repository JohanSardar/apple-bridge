use crate::{
    Error, Gateway, Ingress, IngressFuture, Result, certificate::Certificates, smux, state, stopped,
};
use ab_wire::framing::{RelayChallenge, RelayProof};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Credential {
    pub hostname: String,
    pub credential: String,
}
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("hostname", &self.hostname)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Identity {
    pub key: String,
    pub credential: Credential,
    pub endpoint: String,
}
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("endpoint", &self.endpoint)
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}
impl Identity {
    pub fn signing_key(&self) -> Result<SigningKey> {
        let bytes = STANDARD
            .decode(&self.key)
            .map_err(|_| Error::Contract("invalid device key"))?;
        let pair: [u8; 64] = bytes
            .try_into()
            .map_err(|_| Error::Contract("device key must be Go Ed25519 private key"))?;
        SigningKey::from_keypair_bytes(&pair)
            .map_err(|_| Error::Contract("inconsistent device key"))
    }
    pub async fn proof(&self, client: &reqwest::Client, action: &str) -> Result<RelayProof> {
        let challenge: RelayChallenge = client
            .get(format!("{}/challenge", self.endpoint))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut proof = RelayProof {
            hostname: self.credential.hostname.clone(),
            credential: self.credential.credential.clone(),
            nonce: challenge.nonce,
            signature: String::new(),
        };
        proof.signature = URL_SAFE_NO_PAD.encode(
            self.signing_key()?
                .sign(&proof.signature_input(action))
                .to_bytes(),
        );
        Ok(proof)
    }
    pub async fn control(
        &self,
        client: &reqwest::Client,
        route: &str,
        action: &str,
        mut body: serde_json::Value,
    ) -> Result<()> {
        body["proof"] = serde_json::to_value(self.proof(client, action).await?)?;
        let response = client
            .post(format!("{}{route}", self.endpoint))
            .json(&body)
            .send()
            .await?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(Error::Contract("relay control refused"));
        }
        Ok(())
    }
}

pub async fn enroll(
    home: &Path,
    endpoint: &str,
    zone: &str,
    client: &reqwest::Client,
) -> Result<Identity> {
    if !endpoint.starts_with("https://") {
        return Err(Error::Contract("relay requires HTTPS"));
    }
    let mut identity: Identity = match state::read(home, "relay") {
        Ok(id) => id,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SigningKey::from_bytes(&rand::random());
            let identity = Identity {
                key: STANDARD.encode(key.to_keypair_bytes()),
                credential: Credential::default(),
                endpoint: endpoint.into(),
            };
            state::write(home, "relay", &identity)?;
            identity
        }
        Err(e) => return Err(e),
    };
    let key = identity.signing_key()?;
    if identity.endpoint != endpoint {
        return Err(Error::Contract("relay identity endpoint mismatch"));
    }
    if !identity.credential.hostname.is_empty() {
        return Ok(identity);
    }
    let challenge: RelayChallenge = client
        .get(format!("{endpoint}/challenge"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let message = format!("apple-bridge-relay-enroll\n{zone}\n{}", challenge.nonce);
    identity.credential = client.post(format!("{endpoint}/enroll")).json(&serde_json::json!({
        "public_key": URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()), "nonce": challenge.nonce,
        "signature": URL_SAFE_NO_PAD.encode(key.sign(message.as_bytes()).to_bytes())
    })).send().await?.error_for_status()?.json().await?;
    state::write(home, "relay", &identity)?;
    Ok(identity)
}

#[derive(Debug)]
pub struct Relay {
    pub home: PathBuf,
    pub endpoint: String,
    pub zone: String,
    pub client: reqwest::Client,
    pub certificates: Arc<Certificates>,
    pub tls: Arc<rustls::ClientConfig>,
}
impl Ingress for Relay {
    fn run(&self, gateway: Gateway, mut stop: watch::Receiver<bool>) -> IngressFuture<'_> {
        Box::pin(async move {
            let id = enroll(&self.home, &self.endpoint, &self.zone, &self.client).await?;
            let notifications = crate::ct::Notifications {
                home: self.home.clone(),
            };
            let monitor = async {
                if crate::ct::enabled(&self.home) {
                    let ct_client = crate::ct::PublicCt::new()?;
                    crate::ct::run(
                        &self.home,
                        &id.credential.hostname,
                        &ct_client,
                        &notifications,
                    )
                    .await
                } else {
                    std::future::pending::<Result<()>>().await
                }
            };
            tokio::pin!(monitor);
            let mut delay = Duration::from_millis(100);
            loop {
                tokio::select! {
                    () = stopped(&mut stop) => return Ok(()),
                    result = &mut monitor => return result,
                    result = self.connect(&id, gateway.clone()) => { if let Err(e) = result { eprintln!("relay connection ended: {e}"); } }
                }
                tokio::select! { () = stopped(&mut stop) => return Ok(()), () = tokio::time::sleep(delay) => {} }
                delay = (delay * 2).min(Duration::from_secs(2));
            }
        })
    }
}
impl Relay {
    async fn connect(&self, identity: &Identity, gateway: Gateway) -> Result<()> {
        let proof = identity.proof(&self.client, "tunnel").await?;
        let mut request = format!("{}/tunnel", self.endpoint.replacen("https://", "wss://", 1))
            .into_client_request()?;
        request.headers_mut().insert(
            "X-Bridge-Proof",
            proof
                .encode_header()
                .map_err(|_| Error::Contract("invalid proof"))?
                .parse()
                .map_err(|_| Error::Contract("invalid proof header"))?,
        );
        let (ws, _) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async_tls_with_config(
                request,
                None,
                false,
                Some(tokio_tungstenite::Connector::Rustls(self.tls.clone())),
            ),
        )
        .await
        .map_err(|_| Error::Contract("tunnel timeout"))??;
        let certs = self.certificates.clone();
        let renew = async {
            loop {
                certs.ensure(identity, &self.client).await?;
                tokio::time::sleep(Duration::from_hours(12)).await;
            }
            #[allow(unreachable_code)]
            Ok::<(), Error>(())
        };
        tokio::select! { result = smux::serve(ws, gateway, self.certificates.clone()) => result, result = renew => result }
    }
}
