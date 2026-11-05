use crate::{Error, Result, state};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCAN_INTERVAL: Duration = Duration::from_hours(1);
const RETRY_INTERVAL: Duration = Duration::from_mins(15);
const STALE_AFTER: u64 = 7200;
const OWNER_UNAVAILABLE_AFTER: u64 = 86400;
const PARSED_CACHE_LIMIT: usize = 4096;
const NOT_COMPLETING: &str = "The certificate check has not completed for over a day, so a certificate this Mac did not request could go unnoticed.";
pub const LAG_NOTE: &str = "Public CT aggregation can lag; absence is not an all clear";

pub type CtFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<Certificate>>> + Send + 'a>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Certificate {
    pub fingerprint: String,
    pub names: Vec<String>,
    pub precertificate: bool,
    #[serde(default)]
    pub public_key: String,
}
pub trait CtClient: std::fmt::Debug + Send + Sync {
    fn search<'a>(&'a self, name: &'a str) -> CtFuture<'a>;
}
#[derive(Debug)]
pub struct PublicCt {
    client: reqwest::Client,
    parsed: Mutex<BTreeMap<u64, Certificate>>,
}
impl PublicCt {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            parsed: Mutex::new(BTreeMap::new()),
        })
    }
    fn remembered(&self, id: u64) -> Result<Option<Certificate>> {
        Ok(self
            .parsed
            .lock()
            .map_err(|_| Error::Contract("CT cache lock"))?
            .get(&id)
            .cloned())
    }
    fn keep(&self, id: u64, certificate: Certificate) -> Result<()> {
        let mut parsed = self
            .parsed
            .lock()
            .map_err(|_| Error::Contract("CT cache lock"))?;
        if parsed.len() >= PARSED_CACHE_LIMIT {
            parsed.clear();
        }
        parsed.insert(id, certificate);
        Ok(())
    }
}
impl CtClient for PublicCt {
    fn search<'a>(&'a self, name: &'a str) -> CtFuture<'a> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Hit {
                id: u64,
                #[serde(default)]
                name_value: String,
            }
            let response = self
                .client
                .get("https://crt.sh/")
                .query(&[("q", name), ("output", "json"), ("exclude", "expired")])
                .send()
                .await?
                .error_for_status()?;
            let bytes = bounded(response, 4 * 1024 * 1024).await?;
            let hits: Vec<Hit> = serde_json::from_slice(&bytes)?;
            if hits.len() > 2048 {
                return Err(Error::Contract("CT result limit"));
            }
            let mut result = Vec::new();
            let ids: BTreeSet<_> = hits
                .into_iter()
                .filter(|hit| {
                    hit.name_value.is_empty()
                        || hit
                            .name_value
                            .lines()
                            .any(|line| line.eq_ignore_ascii_case(name))
                })
                .map(|hit| hit.id)
                .collect();
            for id in ids {
                if let Some(known) = self.remembered(id)? {
                    result.push(known);
                    continue;
                }
                let response = self
                    .client
                    .get("https://crt.sh/")
                    .query(&[("d", id)])
                    .send()
                    .await?
                    .error_for_status()?;
                let der = bounded(response, 256 * 1024).await?;
                let certificate = Certificate::from_der(&der)?;
                self.keep(id, certificate.clone())?;
                result.push(certificate);
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
            public_key: hex(&Sha256::digest(parsed.public_key().raw)),
        })
    }
    fn known_to(&self, known: &BTreeSet<String>) -> bool {
        known.contains(&self.fingerprint)
            || (!self.public_key.is_empty() && known.contains(&format!("spki:{}", self.public_key)))
    }
    fn covers(&self, name: &str) -> bool {
        self.names
            .iter()
            .any(|n| affects(n, name) || (name.starts_with("*.") && affects(name, n)))
    }
}
#[must_use]
pub fn key_entry(public_key_der: &[u8]) -> String {
    format!("spki:{}", hex(&Sha256::digest(public_key_der)))
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
    #[serde(default)]
    pub first_checked: u64,
    #[serde(default)]
    pub last_success: u64,
    #[serde(default)]
    pub adopted: usize,
    #[serde(default)]
    pub note: String,
    pub names: BTreeSet<String>,
    pub unknown: Vec<String>,
    pub unexpected: BTreeMap<String, Certificate>,
}
impl Report {
    fn finding_warnings(&self) -> Vec<String> {
        self.unexpected
            .values()
            .map(|cert| {
                format!(
                    "CT unexpected {} {} for {}",
                    if cert.precertificate {
                        "precertificate"
                    } else {
                        "certificate"
                    },
                    cert.fingerprint,
                    cert.names.join(",")
                )
            })
            .collect()
    }
    pub fn warnings(&self, now: u64) -> Vec<String> {
        let mut warnings = self.unknown.clone();
        if self.checked_at == 0
            || now < self.checked_at
            || now.saturating_sub(self.checked_at) > STALE_AFTER
        {
            warnings.push("CT monitor stale or unavailable; certificate safety unknown".into());
        }
        warnings.extend(self.finding_warnings());
        if !warnings.is_empty() && !self.note.is_empty() {
            warnings.push(self.note.clone());
        }
        warnings
    }
    fn owner_warning(&self, now: u64, relay: bool) -> String {
        if !self.unexpected.is_empty() {
            let names = self
                .unexpected
                .values()
                .flat_map(|cert| cert.names.iter().map(String::as_str))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ");
            return format!(
                "A certificate this Mac did not request is public for {names}. Whoever holds it could read this Mac's remote traffic, including through the relay or DNS operator. This is delayed detection, not prevention: the certificate already exists. If it isn't yours, turn Remote off and stop using this relay. Run `bridge doctor` for the fingerprint. If it is yours, run `bridge remote certificate-accept FINGERPRINT`."
            );
        }
        let covered_since = self.last_success.max(self.first_checked);
        if relay
            && covered_since != 0
            && now.saturating_sub(covered_since) > OWNER_UNAVAILABLE_AFTER
        {
            return NOT_COMPLETING.into();
        }
        String::new()
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
    pub status: Option<crate::Gateway>,
}
impl IphonePush for Notifications {
    fn alert<'a>(
        &'a self,
        report: &'a Report,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            state::write(&self.home, "ct-alert", report)?;
            let warnings = report.warnings(now());
            if !warnings.is_empty() {
                eprintln!("{}", warnings.join("; "));
            }
            if let Some(status) = &self.status {
                status.refresh_status();
            }
            Ok(())
        })
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn relay_selected(home: &Path) -> bool {
    state::read::<BTreeMap<String, bool>>(home, "ingresses")
        .is_ok_and(|selected| selected.get("relay") == Some(&true))
}
fn monitor_never_reported(home: &Path, time: u64) -> bool {
    let selected_at = std::fs::metadata(home.join("remote/ingresses.json"))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(time, |age| age.as_secs());
    relay_selected(home) && time.saturating_sub(selected_at) > OWNER_UNAVAILABLE_AFTER
}
pub fn doctor(home: &Path, time: u64) -> Vec<String> {
    let Ok(report) = state::read::<Report>(home, "ct-monitor") else {
        return if monitor_never_reported(home, time) {
            vec![NOT_COMPLETING.into()]
        } else {
            Vec::new()
        };
    };
    if relay_selected(home) {
        report.warnings(time)
    } else {
        report.finding_warnings()
    }
}
#[must_use]
pub fn owner_warning(home: &Path, time: u64) -> String {
    match state::read::<Report>(home, "ct-monitor") {
        Ok(report) => report.owner_warning(time, relay_selected(home)),
        Err(_) if monitor_never_reported(home, time) => NOT_COMPLETING.into(),
        Err(_) => String::new(),
    }
}
pub fn accept(home: &Path, fingerprint: &str) -> Result<()> {
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::Contract(
            "fingerprint must be 64 lowercase hex digits",
        ));
    }
    remember(home, fingerprint)?;
    if let Ok(mut report) = state::read::<Report>(home, "ct-monitor") {
        report.unexpected.remove(fingerprint);
        state::write(home, "ct-monitor", &report)?;
    }
    if let Ok(mut status) = state::read::<state::Status>(home, "status") {
        status.ct_warning = owner_warning(home, now());
        state::write(home, "status", &status)?;
    }
    Ok(())
}
pub fn remember(home: &Path, fingerprint: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(home.join("remote"))?;
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
        first_checked: if previous.first_checked == 0 {
            time
        } else {
            previous.first_checked
        },
        last_success: previous.last_success,
        adopted: previous.adopted,
        note: LAG_NOTE.into(),
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
                        if cert.covers(name) && !cert.known_to(known) {
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
    report.unexpected.retain(|_, cert| !cert.known_to(known));
    if report.unknown.is_empty() {
        report.last_success = time;
    }
    report
}
pub async fn run(home: &Path, assigned: &str, client: &dyn CtClient, push: &dyn IphonePush) {
    loop {
        let report = match scan(home, assigned, client).await {
            Ok(report) => report,
            Err(error) => {
                let mut report: Report = state::read(home, "ct-monitor").unwrap_or_default();
                report.checked_at = now();
                if report.first_checked == 0 {
                    report.first_checked = report.checked_at;
                }
                report.unknown = vec![format!("CT monitor failure; coverage unknown: {error}")];
                report
            }
        };
        let complete = report.unknown.is_empty();
        if let Err(error) = state::write(home, "ct-monitor", &report) {
            eprintln!("CT report persistence failed: {error}");
        }
        if let Err(error) = push.alert(&report).await {
            eprintln!("CT alert delivery failed: {error}");
        }
        tokio::time::sleep(if complete {
            SCAN_INTERVAL
        } else {
            RETRY_INTERVAL
        })
        .await;
    }
}
fn adopt_baseline(home: &Path, report: &mut Report) -> Result<()> {
    if !report.unknown.is_empty() {
        return Ok(());
    }
    let baselined: BTreeSet<String> = match state::read(home, "ct-baseline") {
        Ok(value) => value,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e),
    };
    if report.names.is_subset(&baselined) {
        return Ok(());
    }
    let adopted: Vec<String> = report
        .unexpected
        .values()
        .filter(|cert| !baselined.iter().any(|name| cert.covers(name)))
        .map(|cert| cert.fingerprint.clone())
        .collect();
    for fingerprint in &adopted {
        remember(home, fingerprint)?;
        report.unexpected.remove(fingerprint);
    }
    report.adopted += adopted.len();
    state::write(
        home,
        "ct-baseline",
        &baselined
            .union(&report.names)
            .cloned()
            .collect::<BTreeSet<_>>(),
    )
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
    let mut report = check(client, names, &known, &previous, now()).await;
    adopt_baseline(home, &mut report)?;
    Ok(report)
}
