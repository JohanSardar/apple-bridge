use crate::{
    Credentials, Enrollment, Error, Grant, Principal, atomic_write, effective_grants, fingerprint,
    subset,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, pkcs8::DecodePrivateKey};
use rand::RngCore;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Debug)]
pub struct Authority {
    home: PathBuf,
    pub certificate: Vec<u8>,
    pub(crate) key: Vec<u8>,
    pub fingerprint: String,
}
#[derive(serde::Serialize)]
struct Delegation<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    local_level: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    challenge: &'a str,
    replica_id: &'a str,
    local_principal: &'a str,
    local_grants: &'a [Grant],
    command_id: &'a str,
    request_hash: &'a str,
    iat: i64,
}
impl Authority {
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn open(home: impl AsRef<Path>) -> Result<Self, Error> {
        let home = home.as_ref().to_path_buf();
        let result = Self::open_inner(&home);
        let report = home.join("credential-store-error.json");
        match &result {
            Err(Error::CredentialStore(failure)) => {
                if let Ok(bytes) = serde_json::to_vec(failure) {
                    let _ = atomic_write(&report, &bytes);
                }
            }
            Ok(_) => {
                let _ = fs::remove_file(&report);
            }
            Err(_) => {}
        }
        result
    }
    fn open_inner(home: &Path) -> Result<Self, Error> {
        let home = home.to_path_buf();
        crate::passkeys::validate_home(&home)?;
        fs::create_dir_all(&home)?;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(home.join("credentials.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let store = crate::select(&home, &crate::StoreEnvironment::current())?;
        if !home.join("identity.pem").exists() {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
                .map_err(|e| Error::Identity(e.to_string()))?;
            let mut params = rcgen::CertificateParams::new(vec!["localhost".into()])
                .map_err(|e| Error::Identity(e.to_string()))?;
            params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![
                rcgen::ExtendedKeyUsagePurpose::ServerAuth,
                rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            ];
            let now = time::OffsetDateTime::now_utc();
            params.not_before = now - time::Duration::minutes(1);
            params.not_after = now
                .replace_year(now.year() + 5)
                .or_else(|_| {
                    now.replace_day(28)
                        .and_then(|day| day.replace_year(now.year() + 5))
                        .map(|day| day + time::Duration::days(1))
                })
                .map_err(|e| Error::Identity(e.to_string()))?;
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "Apple Bridge");
            let cert = params
                .self_signed(&key)
                .map_err(|e| Error::Identity(e.to_string()))?;
            store.put("identity-key", key.serialize_pem().as_bytes())?;
            atomic_write(&home.join("identity.pem"), cert.pem().as_bytes())?;
        }
        let key = match store.get("identity-key") {
            Ok(k) => k,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                let k = fs::read(home.join("identity-key.pem"))?;
                store.put("identity-key", &k)?;
                k
            }
            Err(e) => return Err(e),
        };
        let certificate=<rustls::pki_types::CertificateDer<'static> as rustls::pki_types::pem::PemObject>::from_pem_slice(&fs::read(home.join("identity.pem"))?).map_err(|e|Error::Identity(e.to_string()))?.to_vec();
        let key=<rustls::pki_types::PrivateKeyDer<'static> as rustls::pki_types::pem::PemObject>::from_pem_slice(&key).map_err(|e|Error::Identity(e.to_string()))?.secret_der().to_vec();
        let authority = Self {
            home,
            fingerprint: fingerprint(&certificate),
            certificate,
            key,
        };
        authority.client_tls(&authority.fingerprint)?;
        if authority.home.join("identity-key.pem").exists() {
            fs::remove_file(authority.home.join("identity-key.pem"))?;
        }
        drop(lock);
        authority.update(|d| {
            act_without_approval_by_default(d);
            Ok(())
        })?;
        Ok(authority)
    }
    pub fn load(&self) -> Result<Credentials, Error> {
        match fs::read(self.home.join("credentials.json")) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn update<T>(
        &self,
        f: impl FnOnce(&mut Credentials) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.home.join("credentials.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let mut credentials = self.load()?;
        let result = f(&mut credentials)?;
        atomic_write(
            &self.home.join("credentials.json"),
            &serde_json::to_vec(&credentials)?,
        )?;
        drop(lock);
        Ok(result)
    }
    pub fn enrollment(&self) -> Result<Option<Enrollment>, Error> {
        Ok(self.load()?.enrollment)
    }
    pub fn save_enrollment(&self, enrollment: Enrollment) -> Result<(), Error> {
        self.update(|d| {
            if d.enrollment.as_ref().is_some_and(|e| {
                !subset(&e.grants, &enrollment.grants) || !subset(&enrollment.grants, &e.grants)
            }) {
                for p in &mut d.principals {
                    p.revision += 1;
                }
            }
            d.enrollment = Some(enrollment);
            Ok(())
        })
    }
    pub fn create_token(&self, grants: Vec<Grant>, remote: bool) -> Result<String, Error> {
        self.create_token_until(grants, remote, None)
    }
    pub fn create_token_until(
        &self,
        grants: Vec<Grant>,
        remote: bool,
        expires_at: Option<jiff::Timestamp>,
    ) -> Result<String, Error> {
        let mut bytes = [0; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = format!("abt-{}", hex::encode(bytes));
        let id = token_id(&bytes)?;
        self.update(|d| {
            if d.enrollment
                .as_ref()
                .is_some_and(|e| !subset(&grants, &e.grants))
            {
                return Err(Error::Unauthorized);
            }
            let mut principal = Principal {
                id,
                kind: "token".into(),
                grants,
                remote,
                token_hash: fingerprint(token.as_bytes()),
                revision: 1,
                ..Principal::default()
            };
            if let Some(expires_at) = expires_at {
                principal.extra.insert(
                    "expires_at".into(),
                    serde_json::json!(expires_at.to_string()),
                );
            }
            d.principals.push(principal);
            Ok(())
        })?;
        Ok(token)
    }
    pub fn authenticate_token(&self, token: &str) -> Result<Principal, Error> {
        use hmac::Mac;
        if let Ok(principal) = self.authenticate_oauth(token) {
            return Ok(principal);
        }
        let d = self.load()?;
        let hash = fingerprint(token.as_bytes());
        let mut p = d
            .principals
            .into_iter()
            .find(|p| {
                let Ok(mut mac) = hmac::Hmac::<sha2::Sha256>::new_from_slice(hash.as_bytes())
                else {
                    return false;
                };
                mac.update(p.token_hash.as_bytes());
                let tag = mac.finalize().into_bytes();
                let Ok(mut check) = hmac::Hmac::<sha2::Sha256>::new_from_slice(hash.as_bytes())
                else {
                    return false;
                };
                check.update(hash.as_bytes());
                check.verify_slice(&tag).is_ok()
                    && p.active()
                    && ["token", "agent"].contains(&p.kind.as_str())
            })
            .ok_or(Error::Unauthorized)?;
        if let Some(e) = d.enrollment {
            p.grants = effective_grants(&p.grants, &e.grants);
        }
        if p.kind == "agent" && !crate::oauth::recently_used(&p) {
            self.update(|d| {
                let current = d
                    .principals
                    .iter_mut()
                    .find(|c| c.id == p.id && c.token_hash == hash && c.active())
                    .ok_or(Error::Unauthorized)?;
                let now = serde_json::json!(jiff::Timestamp::now().to_string());
                current.extra.insert("last_used".into(), now.clone());
                p.extra.insert("last_used".into(), now);
                Ok(())
            })?;
        }
        Ok(p)
    }
    pub fn client_tls(&self, pin: &str) -> Result<rustls::ClientConfig, Error> {
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| Error::Identity(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(crate::tls::Pin {
            digest: pin.into(),
            accept_unpaired: false,
        }))
        .with_client_auth_cert(
            vec![self.certificate.clone().into()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(self.key.clone()).into(),
        )
        .map_err(|e| Error::Identity(e.to_string()))
    }
    pub fn server_tls(&self, pin: &str) -> Result<rustls::ServerConfig, Error> {
        self.listener_tls(pin, false)
    }
    pub fn gateway_tls(&self) -> Result<rustls::ServerConfig, Error> {
        self.listener_tls("", true)
    }
    fn listener_tls(
        &self,
        pin: &str,
        accept_unpaired: bool,
    ) -> Result<rustls::ServerConfig, Error> {
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| Error::Identity(e.to_string()))?
        .with_client_cert_verifier(Arc::new(crate::tls::Pin {
            digest: pin.into(),
            accept_unpaired,
        }))
        .with_single_cert(
            vec![self.certificate.clone().into()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(self.key.clone()).into(),
        )
        .map_err(|e| Error::Identity(e.to_string()))
    }
    pub(super) fn iphone_signing_key(&self) -> Result<ed25519_dalek::SigningKey, Error> {
        ed25519_dalek::SigningKey::from_pkcs8_der(&self.key)
            .map_err(|e| Error::Identity(e.to_string()))
    }
    pub fn sign_relay_transcript(&self, input: &[u8]) -> Result<[u8; 64], Error> {
        if !input.starts_with(b"apple-bridge-relay/2\0")
            && !input.starts_with(b"apple-bridge-relay/1\0")
        {
            return Err(Error::Invalid);
        }
        Ok(self.iphone_signing_key()?.sign(input).to_bytes())
    }
    pub fn sign_delegation(
        &self,
        principal: &Principal,
        id: &str,
        hash: &str,
        challenge: &str,
    ) -> Result<String, Error> {
        let enrollment = self.enrollment()?.ok_or(Error::Unauthorized)?;
        if !principal.active()
            || !subset(&principal.grants, &enrollment.grants)
            || principal.kind == "agent"
                && !["read", "approve", "full"].contains(&principal.level.as_str())
        {
            return Err(Error::Unauthorized);
        }
        let d = Delegation {
            local_level: if principal.kind == "agent" {
                &principal.level
            } else {
                ""
            },
            challenge,
            replica_id: &enrollment.replica_id,
            local_principal: &principal.id,
            local_grants: &principal.grants,
            command_id: id,
            request_hash: hash,
            iat: jiff::Timestamp::now().as_second(),
        };
        let bytes = ab_wire::gojson::legacy_json(&d, ab_wire::gojson::EscapeMode::Html)
            .map_err(|_| Error::Invalid)?;
        let key = ed25519_dalek::SigningKey::from_pkcs8_der(&self.key)
            .map_err(|e| Error::Identity(e.to_string()))?;
        Ok(format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(key.sign(&bytes).to_bytes())
        ))
    }
}
impl Authority {
    pub fn principal(&self, id: &str) -> Result<Principal, Error> {
        let d = self.load()?;
        let mut p = d
            .principals
            .into_iter()
            .find(|p| p.id == id && p.active())
            .ok_or(Error::Unauthorized)?;
        if let Some(e) = d.enrollment {
            p.grants = effective_grants(&p.grants, &e.grants);
        }
        Ok(p)
    }
    pub fn authorize(&self, p: &Principal, grant: &Grant) -> Result<(), Error> {
        if p.kind == "delegated" {
            if p.level == "read" && grant.as_str().ends_with(":write")
                || !subset(std::slice::from_ref(grant), &p.grants)
            {
                return Err(Error::Unauthorized);
            }
            let parent = self
                .load()?
                .principals
                .into_iter()
                .find(|current| {
                    current.kind == "replica"
                        && current.active()
                        && current.fingerprint == p.fingerprint
                        && subset(&p.grants, &current.grants)
                })
                .ok_or(Error::Unauthorized)?;
            return if subset(std::slice::from_ref(grant), &parent.grants) {
                Ok(())
            } else {
                Err(Error::Unauthorized)
            };
        }
        let current = self.principal(&p.id)?;
        if current.kind == "agent" && current.level == "read" && grant.as_str().ends_with(":write")
            || !subset(std::slice::from_ref(grant), &current.grants)
        {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}

fn token_id(random: &[u8; 32]) -> Result<String, Error> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_millis();
    let seed: [u8; 16] = random[..16].try_into().map_err(|_| Error::Invalid)?;
    let mut number = (millis << 80) | (u128::from_be_bytes(seed) & ((1u128 << 80) - 1));
    let mut chars = [b'0'; 26];
    for c in chars.iter_mut().rev() {
        *c = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ"
            [usize::try_from(number & 31).map_err(|_| Error::Invalid)?];
        number >>= 5;
    }
    Ok(format!(
        "token:{}",
        String::from_utf8(chars.to_vec()).map_err(|_| Error::Invalid)?
    ))
}

const AGENT_DEFAULTS: &str = "agent_level_defaults";
const AGENTS_ACT_AT_ONCE: u64 = 2;

fn act_without_approval_by_default(d: &mut Credentials) {
    if d.extra
        .get(AGENT_DEFAULTS)
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version >= AGENTS_ACT_AT_ONCE)
    {
        return;
    }
    for p in &mut d.principals {
        if p.kind == "agent" && p.level == "approve" {
            p.level = "full".into();
        }
    }
    d.extra
        .insert(AGENT_DEFAULTS.into(), serde_json::json!(AGENTS_ACT_AT_ONCE));
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod agent_default_tests {
    use super::*;

    fn level(authority: &Authority, id: &str) -> String {
        authority
            .agents()
            .expect("agents")
            .into_iter()
            .find(|p| p.id == id)
            .expect("agent")
            .level
    }

    #[test]
    fn apps_connected_before_the_update_stop_asking_once_and_a_later_choice_to_ask_is_kept() {
        let home = tempfile::tempdir().expect("home");
        let authority = Authority::open(home.path()).expect("open");
        let scopes = vec![Grant::parse("reminders:write").expect("grant")];
        let (old, _) = authority
            .create_agent("Old", crate::AgentLevel::Approve, scopes.clone())
            .expect("old");
        let (reader, _) = authority
            .create_agent("Reader", crate::AgentLevel::Read, scopes.clone())
            .expect("reader");
        let revision = authority.agents().expect("agents")[0].revision;
        authority
            .update(|d| {
                d.extra.remove(AGENT_DEFAULTS);
                Ok(())
            })
            .expect("pre-update store");
        drop(authority);
        let authority = Authority::open(home.path()).expect("reopen");
        assert_eq!(level(&authority, &old.id), "full");
        assert_eq!(level(&authority, &reader.id), "read");
        assert_eq!(
            authority.agents().expect("agents")[0].revision,
            revision,
            "existing sessions stay valid"
        );
        let (chosen, _) = authority
            .create_agent("Careful", crate::AgentLevel::Approve, scopes)
            .expect("chosen");
        drop(authority);
        let authority = Authority::open(home.path()).expect("reopen again");
        assert_eq!(level(&authority, &chosen.id), "approve");
    }

    #[test]
    fn the_owner_can_switch_an_app_between_asking_first_and_acting_at_once() {
        let home = tempfile::tempdir().expect("home");
        let authority = Authority::open(home.path()).expect("open");
        let scopes = vec![Grant::parse("reminders:write").expect("grant")];
        let (agent, _) = authority
            .create_agent("App", crate::AgentLevel::DEFAULT, scopes.clone())
            .expect("agent");
        assert_eq!(agent.level, "full");
        let changed = authority
            .set_agent_level(&agent.id, crate::AgentLevel::Approve)
            .expect("ask first");
        assert_eq!(changed.level, "approve");
        assert_eq!(changed.token_hash, "");
        assert_eq!(level(&authority, &agent.id), "approve");
        authority
            .set_agent_level(&agent.id, crate::AgentLevel::Full)
            .expect("act at once");
        assert_eq!(level(&authority, &agent.id), "full");
        let (reader, _) = authority
            .create_agent("Reader", crate::AgentLevel::Read, scopes)
            .expect("reader");
        assert!(matches!(
            authority.set_agent_level(&reader.id, crate::AgentLevel::Full),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            authority.set_agent_level(&agent.id, crate::AgentLevel::Read),
            Err(Error::Invalid)
        ));
        authority.revoke(&agent.id).expect("revoke");
        assert!(matches!(
            authority.set_agent_level(&agent.id, crate::AgentLevel::Approve),
            Err(Error::Unauthorized)
        ));
    }
}
