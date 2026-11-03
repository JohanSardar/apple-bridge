use crate::{Error, Result, state};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub type CtFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<Certificate>>> + Send + 'a>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Certificate {
    pub fingerprint: String,
    pub names: Vec<String>,
    pub precertificate: bool,
}
pub trait CtClient: std::fmt::Debug + Send + Sync {
    fn search<'a>(&'a self, name: &'a str) -> CtFuture<'a>;
}
#[derive(Debug)]
pub struct PublicCt {
    client: reqwest::Client,
}
impl PublicCt {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
}
impl CtClient for PublicCt {
    fn search<'a>(&'a self, name: &'a str) -> CtFuture<'a> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Hit {
                id: u64,
            }
            let response = self
                .client
                .get("https://crt.sh/")
                .query(&[("q", name), ("output", "json")])
                .send()
                .await?
                .error_for_status()?;
            let bytes = bounded(response, 4 * 1024 * 1024).await?;
            let hits: Vec<Hit> = serde_json::from_slice(&bytes)?;
            if hits.len() > 2048 {
                return Err(Error::Contract("CT result limit"));
            }
            let mut result = Vec::new();
            let ids: BTreeSet<_> = hits.into_iter().map(|hit| hit.id).collect();
            for id in ids {
                let response = self
                    .client
                    .get("https://crt.sh/")
                    .query(&[("d", id)])
                    .send()
                    .await?
                    .error_for_status()?;
                let der = bounded(response, 256 * 1024).await?;
                result.push(Certificate::from_der(&der)?);
            }
            Ok(result)
        })
    }
}
async fn bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(Error::Contract("CT response limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
impl Certificate {
    pub fn from_der(der: &[u8]) -> Result<Self> {
        let (remaining, parsed) = x509_parser::parse_x509_certificate(der)
            .map_err(|_| Error::Contract("CT invalid DER"))?;
        if !remaining.is_empty() {
            return Err(Error::Contract("CT trailing DER"));
        }
        let names = parsed
            .subject_alternative_name()
            .map_err(|_| Error::Contract("CT invalid SAN"))?
            .ok_or(Error::Contract("CT missing SAN"))?
            .value
            .general_names
            .iter()
            .filter_map(|n| {
                if let x509_parser::extensions::GeneralName::DNSName(name) = n {
                    Some(name.to_ascii_lowercase())
                } else {
                    None
                }
            })
            .collect();
        Ok(Self {
            fingerprint: hex(&Sha256::digest(der)),
            names,
            precertificate: parsed
                .extensions()
                .iter()
                .any(|e| e.oid.to_id_string() == "1.3.6.1.4.1.11129.2.4.3"),
        })
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub checked_at: u64,
    pub names: BTreeSet<String>,
    pub unknown: Vec<String>,
    pub unexpected: BTreeMap<String, Certificate>,
}
impl Report {
    pub fn warnings(&self, now: u64) -> Vec<String> {
        let mut warnings = self.unknown.clone();
        if self.checked_at == 0
            || now < self.checked_at
            || now.saturating_sub(self.checked_at) > 900
        {
            warnings.push("CT monitor stale or unavailable; certificate safety unknown".into());
        }
        for cert in self.unexpected.values() {
            warnings.push(format!(
                "CT unexpected {} {} for {}",
                if cert.precertificate {
                    "precertificate"
                } else {
                    "certificate"
                },
                cert.fingerprint,
                cert.names.join(",")
            ));
        }
        warnings
    }
}
pub trait IphonePush: std::fmt::Debug + Send + Sync {
    fn alert<'a>(
        &'a self,
        report: &'a Report,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}
#[derive(Debug)]
pub struct Notifications {
    pub home: PathBuf,
}
impl IphonePush for Notifications {
    fn alert<'a>(
        &'a self,
        report: &'a Report,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            state::write(&self.home, "ct-alert", report)?;
            eprintln!("{}", report.warnings(now()).join("; "));
            Ok(())
        })
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
pub fn enabled(home: &Path) -> bool {
    cfg!(feature = "r2-certificates")
        && state::read::<bool>(home, "r2-certificates").unwrap_or(false)
}
pub fn doctor(home: &Path, time: u64) -> Vec<String> {
    if !enabled(home) {
        return Vec::new();
    }
    match state::read::<Report>(home, "ct-monitor") {
        Ok(report) => report.warnings(time),
        Err(_) => vec!["CT monitor unavailable; certificate safety unknown".into()],
    }
}
pub fn remember(home: &Path, fingerprint: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = home.join("remote/ct-known.lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut known: BTreeSet<String> = match state::read(home, "ct-known") {
        Ok(value) => value,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e),
    };
    known.insert(fingerprint.into());
    state::write(home, "ct-known", &known)
}
fn affects(pattern: &str, name: &str) -> bool {
    pattern == name
        || pattern.strip_prefix("*.").is_some_and(|suffix| {
            name.strip_suffix(suffix).is_some_and(|prefix| {
                prefix.ends_with('.') && prefix[..prefix.len() - 1].split('.').count() == 1
            })
        })
}
pub async fn check(
    client: &dyn CtClient,
    names: BTreeSet<String>,
    known: &BTreeSet<String>,
    previous: &Report,
    time: u64,
) -> Report {
    let mut report = Report {
        checked_at: time,
        names,
        unexpected: previous.unexpected.clone(),
        ..Report::default()
    };
    if report.names.is_empty() {
        report.unknown.push("CT has no owned names".into());
    }
    for name in &report.names {
        let queries = std::iter::once(name.clone()).chain(
            name.split_once('.')
                .map(|(_, parent)| format!("*.{parent}")),
        );
        for query in queries {
            match tokio::time::timeout(Duration::from_secs(45), client.search(&query)).await {
                Ok(Ok(certs)) => {
                    for cert in certs {
                        if cert.names.iter().any(|n| {
                            affects(n, name) || (name.starts_with("*.") && affects(name, n))
                        }) && !known.contains(&cert.fingerprint)
                        {
                            report.unexpected.insert(cert.fingerprint.clone(), cert);
                        }
                    }
                }
                _ => report.unknown.push(format!(
                    "CT search unavailable for {query}; coverage unknown"
                )),
            }
        }
    }
    report
        .unexpected
        .retain(|fingerprint, _| !known.contains(fingerprint));
    report
        .unknown
        .push("Public CT aggregation can lag; absence is not an all clear".into());
    report
}
pub async fn run(
    home: &Path,
    assigned: &str,
    client: &dyn CtClient,
    push: &dyn IphonePush,
) -> Result<()> {
    if !enabled(home) {
        return Err(Error::Contract("R2 certificate protection disabled"));
    }
    loop {
        let report = match scan(home, assigned, client).await {
            Ok(report) => report,
            Err(error) => {
                let mut report: Report = state::read(home, "ct-monitor").unwrap_or_default();
                report.checked_at = 0;
                report.unknown = vec![format!("CT monitor failure; coverage unknown: {error}")];
                report
            }
        };
        if let Err(error) = state::write(home, "ct-monitor", &report) {
            eprintln!("CT report persistence failed: {error}");
        }
        if let Err(error) = push.alert(&report).await {
            eprintln!("CT alert delivery failed: {error}");
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}
async fn scan(home: &Path, assigned: &str, client: &dyn CtClient) -> Result<Report> {
    let mut names = BTreeSet::from([assigned.to_ascii_lowercase()]);
    match state::read::<state::Config>(home, "config") {
        Ok(config) if !config.domain.is_empty() => {
            names.insert(config.domain.to_ascii_lowercase());
        }
        Ok(_) => {}
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    match state::read::<BTreeSet<String>>(home, "ct-names") {
        Ok(custom) => names.extend(custom.into_iter().map(|n| n.to_ascii_lowercase())),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let known = match state::read(home, "ct-known") {
        Ok(known) => known,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e),
    };
    let previous = match state::read(home, "ct-monitor") {
        Ok(report) => report,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Report::default(),
        Err(e) => return Err(e),
    };
    Ok(check(client, names, &known, &previous, now()).await)
}
