use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::{Path, PathBuf};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpStream, UnixStream},
};

pub const DEFAULT_SOCKET: &str = "/var/run/tailscaled.socket";
pub const HOST: &str = "local-tailscaled.sock";
pub const CAPABILITY_VERSION: &str = "142";
pub const MACSYS_DIR: &str = "/Library/Tailscale";
const APP_GROUP_MARKER: &str = ".tailscale.ipn.macos/sameuserproof-";
const PROOF_PREFIX: &str = "sameuserproof-";

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

#[derive(Clone, PartialEq, Eq)]
pub enum LocalApi {
    Unix(PathBuf),
    Tcp { port: u16, token: String },
    Discover,
}
impl std::fmt::Debug for LocalApi {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => formatter.debug_tuple("Unix").field(path).finish(),
            Self::Tcp { port, .. } => formatter
                .debug_struct("Tcp")
                .field("port", port)
                .finish_non_exhaustive(),
            Self::Discover => formatter.write_str("Discover"),
        }
    }
}

pub fn parse_lsof(output: &str) -> Option<(u16, String)> {
    output.lines().find_map(|line| {
        let (_, after) = line.split_once(APP_GROUP_MARKER)?;
        parse_proof(after)
    })
}
fn parse_proof(name: &str) -> Option<(u16, String)> {
    let (port, token) = name.split_once('-')?;
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    (!token.is_empty()).then(|| (port, token.to_owned()))
}
pub fn macsys_credentials(dir: &Path) -> Option<(u16, String)> {
    let target = std::fs::read_link(dir.join("ipnport")).ok()?;
    let port_text = target.to_str()?;
    let port = port_text.parse::<u16>().ok().filter(|port| *port != 0)?;
    let token = std::fs::read_to_string(dir.join(format!("{PROOF_PREFIX}{port_text}"))).ok()?;
    let token = token.trim();
    if token.is_empty() || !port_open(port) {
        return None;
    }
    Some((port, token.to_owned()))
}
pub fn group_container_credentials(dir: &Path) -> Option<(u16, String)> {
    let mut found = std::fs::read_dir(dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            parse_proof(name.strip_prefix(PROOF_PREFIX)?)
        })
        .filter(|(port, _)| port_open(*port))
        .collect::<Vec<_>>();
    found.pop()
}
fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_secs(1),
    )
    .is_ok()
}
fn lsof_credentials() -> Option<(u16, String)> {
    let uid = std::process::Command::new("id").arg("-u").output().ok()?;
    let uid = String::from_utf8(uid.stdout).ok()?;
    let binary = if Path::new("/usr/sbin/lsof").exists() {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    };
    let output = std::process::Command::new(binary)
        .args([
            "-n",
            "-a",
            &format!("-u{}", uid.trim()),
            "-c",
            "IPNExtension",
            "-F",
        ])
        .output()
        .ok()?;
    parse_lsof(&String::from_utf8_lossy(&output.stdout))
}
fn app_group_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Group Containers/W5364U7YZB.group.io.tailscale.ipn.macos"),
    )
}
fn macos_credentials() -> Option<(u16, String)> {
    lsof_credentials()
        .or_else(|| macsys_credentials(Path::new(MACSYS_DIR)))
        .or_else(|| app_group_dir().and_then(|dir| group_container_credentials(&dir)))
}

pub async fn discover() -> LocalApi {
    if let Some(socket) = std::env::var_os("APPLE_BRIDGE_TAILSCALE_SOCKET") {
        return LocalApi::Unix(socket.into());
    }
    if cfg!(target_os = "macos")
        && let Ok(Some((port, token))) = tokio::task::spawn_blocking(macos_credentials).await
    {
        return LocalApi::Tcp { port, token };
    }
    LocalApi::Unix(DEFAULT_SOCKET.into())
}

impl LocalApi {
    pub async fn resolve(&self) -> Self {
        match self {
            Self::Discover => discover().await,
            other => other.clone(),
        }
    }
    pub fn authorization(&self) -> Option<String> {
        match self {
            Self::Tcp { token, .. } => {
                Some(format!("Basic {}", STANDARD.encode(format!(":{token}"))))
            }
            _ => None,
        }
    }
    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        if_match: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response> {
        let api = self.resolve().await;
        let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));
        let (client, base) = match &api {
            Self::Unix(socket) => (
                builder.unix_socket(socket.as_path()).build()?,
                format!("http://{HOST}/localapi/v0"),
            ),
            Self::Tcp { port, .. } => (
                builder.build()?,
                format!("http://127.0.0.1:{port}/localapi/v0"),
            ),
            Self::Discover => return Err(Error::Contract("unresolved Tailscale LocalAPI")),
        };
        let mut request = client
            .request(method, format!("{base}/{path}"))
            .header("Host", HOST)
            .header("Tailscale-Cap", CAPABILITY_VERSION);
        if let Some(authorization) = api.authorization() {
            request = request.header("Authorization", authorization);
        }
        if let Some(etag) = if_match {
            request = request.header("If-Match", etag);
        }
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        Ok(request.send().await?)
    }
    pub async fn connect(&self) -> Result<(Box<dyn Stream>, Option<String>)> {
        let api = self.resolve().await;
        let authorization = api.authorization();
        match api {
            Self::Unix(socket) => Ok((Box::new(UnixStream::connect(socket).await?), authorization)),
            Self::Tcp { port, .. } => Ok((
                Box::new(TcpStream::connect(("127.0.0.1", port)).await?),
                authorization,
            )),
            Self::Discover => Err(Error::Contract("unresolved Tailscale LocalAPI")),
        }
    }
}

pub fn civil(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86400);
    let seconds = timestamp.rem_euclid(86400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

pub async fn certificate_expiry(host: &str, port: u16) -> Result<String> {
    use std::sync::Arc;
    #[allow(unused_mut)]
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    #[allow(unused_mut)]
    let mut target = format!("{host}:{port}");
    #[cfg(feature = "lab-harness")]
    {
        use rustls::pki_types::pem::PemObject;
        if let Some(address) = std::env::var_os("APPLE_BRIDGE_REMOTE_PROBE_ADDRESS") {
            let address = address.to_string_lossy().into_owned();
            let loopback = address
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|address| address.ip().is_loopback());
            if !loopback {
                return Err(Error::Contract("invalid lab probe address"));
            }
            target = address;
        }
        if let Some(path) = std::env::var_os("APPLE_BRIDGE_LAB_TLS_ROOT") {
            use rustls::pki_types::pem::PemObject;
            let pem = std::fs::read(path)?;
            for certificate in <rustls::pki_types::CertificateDer as rustls::pki_types::pem::PemObject>::pem_slice_iter(&pem)
                .filter_map(std::result::Result::ok)
            {
                roots.add(certificate)?;
            }
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| Error::Contract("invalid certificate host"))?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let socket = TcpStream::connect(target).await?;
        let tls = connector.connect(name, socket).await?;
        let leaf = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(<[rustls::pki_types::CertificateDer<'_>]>::first)
            .ok_or(Error::Contract("missing public certificate"))?;
        let (_, certificate) = x509_parser::parse_x509_certificate(leaf.as_ref())
            .map_err(|_| Error::Contract("invalid public certificate"))?;
        Ok(civil(certificate.validity().not_after.timestamp()))
    })
    .await
    .map_err(|_| Error::Contract("certificate probe timeout"))?
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn civil_matches_known_dates() {
        assert_eq!(civil(0), "1970-01-01T00:00:00Z");
        assert_eq!(civil(1_799_286_713), "2027-01-07T01:51:53Z");
        assert_eq!(civil(951_782_400), "2000-02-29T00:00:00Z");
    }
    #[test]
    fn lsof_output_selects_app_credentials() {
        let output = "p123\nfcwd\nn/Users/a/Library/Group Containers/W5364U7YZB.group.io.tailscale.ipn.macos/sameuserproof-52632-7ff97473cb233941db34\nn/other\n";
        assert_eq!(
            parse_lsof(output),
            Some((52632, "7ff97473cb233941db34".into()))
        );
        assert_eq!(parse_lsof("n/tmp/sameuserproof-1-a"), None);
        assert_eq!(
            parse_lsof("n/x.tailscale.ipn.macos/sameuserproof-notaport-abc"),
            None
        );
        assert_eq!(
            parse_lsof("n/x.tailscale.ipn.macos/sameuserproof-0-abc"),
            None
        );
    }
    #[test]
    fn macsys_requires_live_port() {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::os::unix::fs::symlink(port.to_string(), dir.path().join("ipnport")).unwrap();
        std::fs::write(dir.path().join(format!("sameuserproof-{port}")), "abc123\n").unwrap();
        assert_eq!(
            macsys_credentials(dir.path()),
            Some((port, "abc123".into()))
        );
        drop(listener);
        assert_eq!(macsys_credentials(dir.path()), None);
    }
    #[test]
    fn group_container_requires_live_port() {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(dir.path().join(format!("sameuserproof-{port}-tok")), "").unwrap();
        assert_eq!(
            group_container_credentials(dir.path()),
            Some((port, "tok".into()))
        );
        drop(listener);
        assert_eq!(group_container_credentials(dir.path()), None);
    }
    #[test]
    fn basic_authorization_has_empty_user() {
        let api = LocalApi::Tcp {
            port: 1,
            token: "tok".into(),
        };
        assert_eq!(api.authorization().as_deref(), Some("Basic OnRvaw=="));
        assert!(!format!("{api:?}").contains("tok"));
    }
}
