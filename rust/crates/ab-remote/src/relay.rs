use crate::{
    Error, Gateway, Ingress, IngressFuture, Progress, Result,
    certificate::Certificates,
    dns::{self, Resolver},
    smux, state, stopped,
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

#[derive(Debug, Default, Deserialize)]
pub struct RelayInfo {
    #[serde(default)]
    pub zone: String,
    #[serde(default)]
    pub accounts: String,
}
pub async fn info(endpoint: &str, client: &reqwest::Client) -> Result<RelayInfo> {
    Ok(client
        .get(format!("{endpoint}/info"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
fn load_identity(home: &Path, endpoint: &str) -> Result<Identity> {
    match state::read(home, "relay") {
        Ok(identity) => Ok(identity),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SigningKey::from_bytes(&rand::random());
            let identity = Identity {
                key: STANDARD.encode(key.to_keypair_bytes()),
                credential: Credential::default(),
                endpoint: endpoint.into(),
            };
            state::write(home, "relay", &identity)?;
            Ok(identity)
        }
        Err(e) => Err(e),
    }
}
enum Enrollment {
    Enrolled(Credential),
    NotListed,
}
async fn enroll_by_key(
    identity: &Identity,
    zone: &str,
    client: &reqwest::Client,
) -> Result<Enrollment> {
    let key = identity.signing_key()?;
    let challenge: RelayChallenge = client
        .get(format!("{}/challenge", identity.endpoint))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let message = format!("apple-bridge-relay-enroll\n{zone}\n{}", challenge.nonce);
    let response = client.post(format!("{}/enroll", identity.endpoint)).json(&serde_json::json!({
        "public_key": URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()), "nonce": challenge.nonce,
        "signature": URL_SAFE_NO_PAD.encode(key.sign(message.as_bytes()).to_bytes())
    })).send().await?;
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Ok(Enrollment::NotListed);
    }
    Ok(Enrollment::Enrolled(
        response.error_for_status()?.json().await?,
    ))
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
    let mut identity = load_identity(home, endpoint)?;
    if identity.endpoint != endpoint {
        return Err(Error::Contract("relay identity endpoint mismatch"));
    }
    if !identity.credential.hostname.is_empty() {
        return Ok(identity);
    }
    match enroll_by_key(&identity, zone, client).await? {
        Enrollment::Enrolled(credential) => {
            identity.credential = credential;
            state::write(home, "relay", &identity)?;
            Ok(identity)
        }
        Enrollment::NotListed => Err(Error::Remote(format!(
            "This relay does not list this Mac. Add this key to its owner-keys file, then turn Remote on again: {}",
            URL_SAFE_NO_PAD.encode(identity.signing_key()?.verifying_key().as_bytes())
        ))),
    }
}

pub fn public_key(home: &Path, endpoint: &str) -> Result<String> {
    let identity = load_identity(home, endpoint)?;
    Ok(URL_SAFE_NO_PAD.encode(identity.signing_key()?.verifying_key().as_bytes()))
}

#[derive(Deserialize)]
struct GrantStart {
    polling_secret: String,
    user_code: String,
    fingerprint: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    interval: u64,
    expires_in: u64,
}
#[derive(Deserialize)]
struct GrantProof {
    transaction: String,
    nonce: String,
}

fn computer_name() -> String {
    std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "Mac".to_owned(), |name| name.trim().to_owned())
}

async fn link_to_account(
    home: &Path,
    identity: &mut Identity,
    accounts: &str,
    client: &reqwest::Client,
    gateway: &Gateway,
    stop: &mut watch::Receiver<bool>,
) -> Result<bool> {
    let key = identity.signing_key()?;
    let public_key = URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes());
    let start: GrantStart = client
        .post(format!("{accounts}/v1/device-start"))
        .json(&serde_json::json!({"public_key": public_key, "name": computer_name()}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let link = if start.verification_uri_complete.is_empty() {
        start.verification_uri.clone()
    } else {
        start.verification_uri_complete.clone()
    };
    gateway.progress(
        "relay",
        Progress {
            state: "pending",
            message: format!(
                "Link this Mac at {}\nCode: {}\nKey: {}",
                start.verification_uri, start.user_code, start.fingerprint
            ),
            pending_link: "account link pending".into(),
            last_error: String::new(),
            link,
        },
    );
    let mut interval = start.interval.max(1);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(start.expires_in);
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            () = stopped(stop) => return Ok(false),
            () = tokio::time::sleep(Duration::from_secs(interval)) => {}
        }
        let Ok(response) = client
            .post(format!("{accounts}/v1/device-poll"))
            .json(&serde_json::json!({"polling_secret": start.polling_secret}))
            .send()
            .await
        else {
            continue;
        };
        match response.status() {
            reqwest::StatusCode::ACCEPTED => {}
            reqwest::StatusCode::TOO_MANY_REQUESTS => interval = (interval + 5).min(60),
            status if status.is_success() => {
                let proof: GrantProof = response.json().await?;
                let message = ab_wire::framing::device_link_message(
                    accounts,
                    &proof.transaction,
                    &proof.nonce,
                    &public_key,
                );
                identity.credential = client
                    .post(format!("{accounts}/v1/device-complete"))
                    .json(&serde_json::json!({
                        "polling_secret": start.polling_secret,
                        "signature": URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes()),
                    }))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                state::write(home, "relay", identity)?;
                gateway.clear_progress("relay");
                return Ok(true);
            }
            _ => {
                return Err(Error::Remote(
                    "This link expired. Turn Remote on again.".into(),
                ));
            }
        }
    }
    Err(Error::Remote(
        "This link expired. Turn Remote on again.".into(),
    ))
}

#[derive(Debug)]
pub struct Relay {
    pub home: PathBuf,
    pub endpoint: String,
    pub zone: String,
    pub client: reqwest::Client,
    pub certificates: Arc<Certificates>,
    pub tls: Arc<rustls::ClientConfig>,
    pub domain: String,
    pub resolver: Option<Arc<dyn Resolver>>,
    pub domain_poll: Duration,
}
impl Ingress for Relay {
    fn run(&self, gateway: Gateway, mut stop: watch::Receiver<bool>) -> IngressFuture<'_> {
        Box::pin(async move {
            let Some(id) = self.identity(&gateway, &mut stop).await? else {
                return Ok(());
            };
            let host = if self.domain.is_empty() {
                id.credential.hostname.clone()
            } else {
                if !self.claim(&id, &gateway, &mut stop).await? {
                    return Ok(());
                }
                self.domain.clone()
            };
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
                    result = self.connect(&id, &host, gateway.clone()) => { if let Err(e) = result { eprintln!("relay connection ended: {e}"); } }
                }
                tokio::select! { () = stopped(&mut stop) => return Ok(()), () = tokio::time::sleep(delay) => {} }
                delay = (delay * 2).min(Duration::from_secs(2));
            }
        })
    }
}
impl Relay {
    async fn identity(
        &self,
        gateway: &Gateway,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Option<Identity>> {
        if !self.endpoint.starts_with("https://") {
            return Err(Error::Contract("relay requires HTTPS"));
        }
        let mut identity = load_identity(&self.home, &self.endpoint)?;
        if identity.endpoint != self.endpoint {
            return Err(Error::Contract("relay identity endpoint mismatch"));
        }
        if !identity.credential.hostname.is_empty() {
            return Ok(Some(identity));
        }
        let info = info(&self.endpoint, &self.client).await.unwrap_or_default();
        let zone = if self.zone.is_empty() {
            &info.zone
        } else {
            &self.zone
        };
        match enroll_by_key(&identity, zone, &self.client).await? {
            Enrollment::Enrolled(credential) => {
                identity.credential = credential;
                state::write(&self.home, "relay", &identity)?;
                Ok(Some(identity))
            }
            Enrollment::NotListed if !info.accounts.is_empty() => {
                let linked = link_to_account(
                    &self.home,
                    &mut identity,
                    &info.accounts,
                    &self.client,
                    gateway,
                    stop,
                )
                .await?;
                Ok(linked.then_some(identity))
            }
            Enrollment::NotListed => Err(Error::Remote(format!(
                "This relay does not list this Mac. Add this key to its owner-keys file, then turn Remote on again: {}",
                URL_SAFE_NO_PAD.encode(identity.signing_key()?.verifying_key().as_bytes())
            ))),
        }
    }
    async fn claim(
        &self,
        identity: &Identity,
        gateway: &Gateway,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<bool> {
        let system;
        let resolver: &dyn Resolver = if let Some(resolver) = &self.resolver {
            resolver.as_ref()
        } else {
            system = dns::Hickory::system()?;
            &system
        };
        loop {
            let missing =
                dns::missing_records(resolver, &self.domain, &identity.credential.hostname).await;
            if missing.is_empty() {
                break;
            }
            gateway.progress(
                "relay",
                Progress {
                    state: "pending",
                    message: format!("Missing DNS records: {}", missing.join("; ")),
                    pending_link: String::new(),
                    last_error: String::new(),
                    link: String::new(),
                },
            );
            tokio::select! {
                () = stopped(stop) => return Ok(false),
                () = tokio::time::sleep(self.domain_poll) => {}
            }
        }
        identity
            .control(
                &self.client,
                "/domain",
                &format!("domain:{}", self.domain),
                serde_json::json!({"domain": self.domain}),
            )
            .await?;
        gateway.clear_progress("relay");
        Ok(true)
    }
    async fn connect(&self, identity: &Identity, host: &str, gateway: Gateway) -> Result<()> {
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
        self.certificates
            .ensure(identity, host, &self.client)
            .await?;
        let _active = gateway.ready("relay", host);
        let certs = self.certificates.clone();
        let renew = async {
            loop {
                certs.ensure(identity, host, &self.client).await?;
                tokio::time::sleep(Duration::from_hours(12)).await;
            }
            #[allow(unreachable_code)]
            Ok::<(), Error>(())
        };
        tokio::select! { result = smux::serve(ws, gateway, self.certificates.clone()) => result, result = renew => result }
    }
}
