use ab_trust::{
    Error,
    oauth::{Client, display_name, redirect_matches, valid_redirect},
};
use std::{
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    time::{Duration, Instant},
};
fn metadata_url(raw: &str) -> Result<url::Url, Error> {
    let u = url::Url::parse(raw).map_err(|_| Error::Invalid)?;
    if raw.len() > 2048
        || u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || raw.contains('#')
        || ["", "/"].contains(&u.path())
        || raw
            .split('/')
            .any(|p| [".", "..", "%2e", "%2e%2e"].contains(&p.to_ascii_lowercase().as_str()))
    {
        return Err(Error::Invalid);
    }
    Ok(u)
}
fn public(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    let denied = [
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "192.0.2.0/24",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "224.0.0.0/3",
        "192.88.99.0/24",
        "192.0.0.0/24",
        "198.18.0.0/15",
        "2001::/23",
        "3fff::/20",
        "2001:db8::/32",
        "64:ff9b::/96",
        "64:ff9b:1::/48",
        "2002::/16",
    ];
    if let IpAddr::V6(v) = ip
        && v.segments()[0] & 0xe000 != 0x2000
    {
        return false;
    }
    !denied
        .iter()
        .any(|s| s.parse::<ipnet::IpNet>().is_ok_and(|net| net.contains(&ip)))
}
#[derive(Debug, Default)]
pub(crate) struct Fetcher {
    cache: std::sync::Mutex<std::collections::HashMap<String, (Client, Duration)>>,
}
impl Fetcher {
    pub(crate) fn fetch(&self, id: &str, redirect: &str) -> Result<Client, Error> {
        metadata_url(id)?;
        if let Some((client, _)) = self
            .cache
            .lock()
            .map_err(|_| Error::Unavailable)?
            .get(id)
            .filter(|(_, expires)| *expires > clock())
        {
            return if client
                .redirect_uris
                .iter()
                .any(|u| redirect_matches(u, redirect))
            {
                Ok(client.clone())
            } else {
                Err(Error::Invalid)
            };
        }
        let mut url = metadata_url(id)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        for hop in 0..5 {
            let host = url.host_str().ok_or(Error::Invalid)?;
            let port = url.port_or_known_default().ok_or(Error::Invalid)?;
            let resolved = addresses(host, port)?;
            if resolved.is_empty() || resolved.iter().any(|a| !public(a.ip())) {
                return Err(Error::Invalid);
            }
            let checked = addresses(host, port)?;
            if checked.is_empty() || checked.iter().any(|a| !public(a.ip())) {
                return Err(Error::Invalid);
            }
            let builder = reqwest::blocking::Client::builder();
            let targets = checked;
            #[cfg(feature = "lab-harness")]
            let (builder, targets) = fixture_transport(builder, targets, host, port)?;
            let client = builder
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(
                    deadline
                        .checked_duration_since(Instant::now())
                        .ok_or(Error::Invalid)?,
                )
                .resolve_to_addrs(host, &targets)
                .build()?;
            let response = client.get(url.clone()).send()?;
            if response.status().is_redirection() {
                if hop == 4 {
                    return Err(Error::Invalid);
                }
                let next = url
                    .join(
                        response
                            .headers()
                            .get(reqwest::header::LOCATION)
                            .and_then(|v| v.to_str().ok())
                            .ok_or(Error::Invalid)?,
                    )
                    .map_err(|_| Error::Invalid)?;
                url = metadata_url(next.as_str())?;
                continue;
            }
            if response.status() != 200 {
                return Err(Error::Invalid);
            }
            let headers = response.headers().clone();
            let mut raw = Vec::new();
            response.take(5121).read_to_end(&mut raw)?;
            if raw.len() > 5120 {
                return Err(Error::Invalid);
            }
            let mut c: Client = serde_json::from_slice(&raw)?;
            if c.client_id != id
                || !c
                    .redirect_uris
                    .iter()
                    .any(|u| redirect_matches(u, redirect))
                || !c.redirect_uris.iter().all(|u| valid_redirect(u))
            {
                return Err(Error::Invalid);
            }
            c.client_name = display_name(&c);
            let ttl = cache_ttl(&headers);
            if ttl > 0 {
                let mut cache = self.cache.lock().map_err(|_| Error::Unavailable)?;
                if cache.len() >= 1000 {
                    cache.clear();
                }
                cache.insert(id.into(), (c.clone(), clock() + Duration::from_secs(ttl)));
            }
            return Ok(c);
        }
        Err(Error::Invalid)
    }
}

fn cache_ttl(headers: &reqwest::header::HeaderMap) -> u64 {
    let mut ttl = 0u64;
    for part in headers
        .get_all(reqwest::header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
    {
        let (key, value) = part.trim().split_once('=').unwrap_or((part.trim(), ""));
        if key.eq_ignore_ascii_case("no-store") || key.eq_ignore_ascii_case("no-cache") {
            ttl = 0;
            break;
        }
        if key.eq_ignore_ascii_case("max-age") {
            ttl = value.trim_matches('"').parse::<u32>().map_or(0, u64::from);
        }
    }
    let age = headers
        .get(reqwest::header::AGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(0, u64::from);
    ttl = ttl.saturating_sub(age);

    ttl
}

#[cfg(feature = "lab-harness")]
fn fixture<T: serde::de::DeserializeOwned>(path: &str, host: &str) -> Result<T, Error> {
    let base = std::env::var("APPLE_BRIDGE_LAB_CIMD").map_err(|_| Error::Invalid)?;
    Ok(reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()?
        .get(format!("{base}{path}"))
        .query(&[("host", host)])
        .send()?
        .json()?)
}
fn addresses(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, Error> {
    #[cfg(feature = "lab-harness")]
    if std::env::var("APPLE_BRIDGE_LAB_CIMD").is_ok() {
        let ips: Vec<String> = fixture("/resolve", host)?;
        return ips
            .iter()
            .map(|ip| {
                ip.parse::<IpAddr>()
                    .map(|ip| std::net::SocketAddr::new(ip, port))
                    .map_err(|_| Error::Invalid)
            })
            .collect();
    }
    Ok((host, port).to_socket_addrs()?.collect())
}
fn clock() -> Duration {
    #[cfg(feature = "lab-harness")]
    if std::env::var("APPLE_BRIDGE_LAB_CIMD").is_ok()
        && let Ok(seconds) = fixture::<u64>("/clock", "")
    {
        return Duration::from_secs(seconds);
    }
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

#[cfg(feature = "lab-harness")]
fn fixture_transport(
    mut builder: reqwest::blocking::ClientBuilder,
    mut targets: Vec<std::net::SocketAddr>,
    host: &str,
    port: u16,
) -> Result<(reqwest::blocking::ClientBuilder, Vec<std::net::SocketAddr>), Error> {
    if std::env::var("APPLE_BRIDGE_LAB_CIMD").is_ok() {
        let target: String = fixture("/dial", &format!("{host}:{port}"))?;
        targets = vec![target.parse().map_err(|_| Error::Invalid)?];
        let pem =
            std::fs::read(std::env::var("APPLE_BRIDGE_LAB_CIMD_CA").map_err(|_| Error::Invalid)?)?;
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    Ok((builder, targets))
}
