use crate::{
    Error, Gateway, Ingress, IngressFuture, Progress, Result, localapi::LocalApi, state, stopped,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
    task::JoinSet,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Exposure {
    #[default]
    Funnel,
    Tailnet,
}
impl Exposure {
    #[must_use]
    pub const fn ingress(self) -> &'static str {
        match self {
            Self::Funnel => "system",
            Self::Tailnet => "tailnet",
        }
    }
}
#[derive(Debug)]
pub struct Tailscale {
    pub home: PathBuf,
    pub api: LocalApi,
    pub exposure: Exposure,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Lease {
    pub host: String,
    pub port: u16,
    pub target: String,
    pub absent: BTreeMap<String, bool>,
    #[serde(default)]
    pub private: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tailnet: bool,
}
fn section<'a>(value: &'a Value, key: &str) -> Result<Option<&'a serde_json::Map<String, Value>>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        _ => Err(Error::Contract("invalid Serve section")),
    }
}
fn port_used(value: &Value, port: u16) -> Result<bool> {
    let p = port.to_string();
    for key in ["TCP", "Web", "AllowFunnel"] {
        if let Some(map) = section(value, key)?
            && map.keys().any(|k| *k == p || k.ends_with(&format!(":{p}")))
        {
            return Ok(true);
        }
    }
    for key in ["Foreground", "Services"] {
        if let Some(map) = section(value, key)? {
            for nested in map.values() {
                if !nested.is_object() || port_used(nested, port)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
impl Tailscale {
    pub fn local(home: PathBuf, socket: &std::path::Path) -> Result<Self> {
        Ok(Self {
            home,
            api: LocalApi::Unix(socket.to_owned()),
            exposure: Exposure::Funnel,
        })
    }
    pub fn tcp(home: PathBuf, port: u16, token: String) -> Self {
        Self {
            home,
            api: LocalApi::Tcp { port, token },
            exposure: Exposure::Funnel,
        }
    }
    pub fn system(home: PathBuf) -> Self {
        Self {
            home,
            api: LocalApi::Discover,
            exposure: Exposure::Funnel,
        }
    }
    #[must_use]
    pub fn tailnet(home: PathBuf) -> Self {
        Self {
            home,
            api: LocalApi::Discover,
            exposure: Exposure::Tailnet,
        }
    }
    #[must_use]
    pub fn exposed(mut self, exposure: Exposure) -> Self {
        self.exposure = exposure;
        self
    }
    async fn get(&self) -> Result<(Value, String)> {
        let response = self
            .api
            .request(reqwest::Method::GET, "serve-config", None, None)
            .await?
            .error_for_status()?;
        let etag = response
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .ok_or(Error::Contract("Serve config missing ETag"))?
            .to_owned();
        let mut value: Value = response.json().await?;
        if value.is_null() {
            value = json!({});
        }
        if !value.is_object() {
            return Err(Error::Contract("invalid Serve config"));
        }
        Ok((value, etag))
    }
    async fn set(&self, value: &Value, etag: &str) -> Result<bool> {
        let response = self
            .api
            .request(
                reqwest::Method::POST,
                "serve-config",
                Some(etag),
                Some(serde_json::to_vec(value)?),
            )
            .await?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Ok(false);
        }
        response.error_for_status()?;
        Ok(true)
    }
    async fn feature_link(&self, feature: &str) -> String {
        let response = self
            .api
            .request(
                reqwest::Method::POST,
                &format!("query-feature?feature={feature}"),
                None,
                Some(Vec::new()),
            )
            .await;
        let value: Value = match response {
            Ok(response) => response.json().await.unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };
        value["URL"]
            .as_str()
            .filter(|url| url.starts_with("https://login.tailscale.com/"))
            .unwrap_or_default()
            .to_owned()
    }
    async fn await_ready(
        &self,
        gateway: &Gateway,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<bool> {
        let interval = std::time::Duration::from_millis(
            std::env::var("APPLE_BRIDGE_REMOTE_RECHECK_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|millis| *millis > 0)
                .unwrap_or(5000),
        );
        let mut login_started = false;
        loop {
            let status: Value = self
                .api
                .request(reqwest::Method::GET, "status?peers=false", None, None)
                .await
                .map_err(|_| Error::Contract("Tailscale approval check failed"))?
                .error_for_status()?
                .json()
                .await?;
            if status["BackendState"] == "NeedsLogin"
                && status["AuthURL"].as_str().is_none_or(str::is_empty)
                && !login_started
            {
                login_started = true;
                let _ = self
                    .api
                    .request(reqwest::Method::POST, "login-interactive", None, None)
                    .await;
                continue;
            }
            let capabilities = &status["Self"]["CapMap"];
            let progress = if status["BackendState"] != "Running" {
                Some(Progress {
                    state: "login_pending",
                    message: "Waiting for Tailscale login".into(),
                    pending_link: "login pending".into(),
                    last_error: String::new(),
                    link: status["AuthURL"].as_str().unwrap_or_default().into(),
                })
            } else if self.exposure == Exposure::Tailnet && capabilities.get("https").is_none() {
                let link = self.feature_link("serve").await;
                Some(Progress {
                    state: "https_pending",
                    message: "Waiting for HTTPS certificates to be turned on in Tailscale".into(),
                    pending_link: "HTTPS approval pending".into(),
                    last_error: String::new(),
                    link: if link.is_empty() {
                        "https://login.tailscale.com/admin/dns".into()
                    } else {
                        link
                    },
                })
            } else if self.exposure == Exposure::Funnel
                && (capabilities.get("https").is_none() || capabilities.get("funnel").is_none())
            {
                Some(Progress {
                    state: "funnel_pending",
                    message: "Waiting for Funnel approval".into(),
                    pending_link: "Funnel approval pending".into(),
                    last_error: String::new(),
                    link: self.feature_link("funnel").await,
                })
            } else {
                None
            };
            let Some(progress) = progress else {
                gateway.clear_progress(self.exposure.ingress());
                return Ok(true);
            };
            gateway.progress(self.exposure.ingress(), progress);
            tokio::select! {
                () = stopped(stop) => return Ok(false),
                () = tokio::time::sleep(interval) => {}
            }
        }
    }
    pub async fn add(&self, target: &str) -> Result<Lease> {
        self.add_mode(target, false).await
    }
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn add_mode(&self, target: &str, private: bool) -> Result<Lease> {
        let tailnet = !private && self.exposure == Exposure::Tailnet;
        let status: Value = self
            .api
            .request(reqwest::Method::GET, "status?peers=false", None, None)
            .await?
            .error_for_status()?
            .json()
            .await?;
        if status["BackendState"] != "Running" {
            return Err(Error::Contract("Tailscale is not running"));
        }
        let host = status["Self"]["DNSName"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or(Error::Contract("missing Tailscale DNS name"))?
            .trim_end_matches('.');
        let caps = &status["Self"]["CapMap"];
        let has = |name: &str| caps.get(name).is_some();
        if tailnet && !has("https") {
            return Err(Error::Contract("Tailscale HTTPS capability unavailable"));
        }
        if !private && !tailnet && (!has("https") || !has("funnel")) {
            return Err(Error::Contract(
                "Tailscale HTTPS/Funnel capability unavailable",
            ));
        }
        let allowed = caps
            .as_object()
            .into_iter()
            .flat_map(|map| map.keys())
            .filter_map(|k| k.strip_prefix("https://tailscale.com/cap/funnel-ports?ports="))
            .collect::<Vec<_>>();
        for _ in 0..5 {
            let (mut config, etag) = self.get().await?;
            let mut selected = None;
            for port in if private {
                vec![7780_u16]
            } else if tailnet {
                vec![8443_u16, 9443, 4443]
            } else {
                vec![443_u16, 8443, 10000]
            } {
                let permitted = allowed.iter().any(|list| list.split(',').any(|range| {
                    if let Some((a,b)) = range.split_once('-') { matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a),Ok(b)) if (a..=b).contains(&port)) } else { range.parse::<u16>() == Ok(port) }
                }));
                if (private || tailnet || permitted) && !port_used(&config, port)? {
                    selected = Some(port);
                    break;
                }
            }
            let port = selected.ok_or(Error::Contract(if tailnet {
                "no free Tailscale HTTPS port"
            } else {
                "no free allowed Funnel port"
            }))?;
            let hp = format!("{host}:{port}");
            let mut lease = Lease {
                host: host.into(),
                port,
                target: target.into(),
                absent: BTreeMap::new(),
                private,
                tailnet,
            };
            let mappings = if private {
                vec![("TCP", port.to_string(), json!({"TCPForward":target}))]
            } else if tailnet {
                vec![
                    ("TCP", port.to_string(), json!({"HTTPS":true})),
                    ("Web", hp, json!({"Handlers":{"/":{"Proxy":target}}})),
                ]
            } else {
                vec![
                    ("TCP", port.to_string(), json!({"HTTPS":true})),
                    (
                        "Web",
                        hp.clone(),
                        json!({"Handlers":{"/":{"Proxy":target}}}),
                    ),
                    ("AllowFunnel", hp, json!(true)),
                ]
            };
            for (section_name, index, value) in mappings {
                lease
                    .absent
                    .insert(section_name.into(), config.get(section_name).is_none());
                if config.get(section_name).is_none_or(Value::is_null) {
                    config[section_name] = json!({});
                }
                config[section_name][index] = value;
            }
            state::write(&self.home, "lease", &lease)?;
            if self.set(&config, &etag).await? {
                return Ok(lease);
            }
        }
        Err(Error::Contract(
            "Serve config changed during five ETag attempts",
        ))
    }
    pub async fn remove(&self) -> Result<()> {
        let lease: Lease = match state::read(&self.home, "lease") {
            Ok(lease) => lease,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let hp = format!("{}:{}", lease.host, lease.port);
        if lease.private {
            for _ in 0..5 {
                let (mut config, etag) = self.get().await?;
                let key = lease.port.to_string();
                if config["TCP"].get(&key).is_none() {
                    break;
                }
                if config["TCP"][&key] != json!({"TCPForward":lease.target}) {
                    return Err(Error::Contract("owned private mapping changed"));
                }
                config["TCP"]
                    .as_object_mut()
                    .ok_or(Error::Contract("invalid TCP section"))?
                    .remove(&key);
                if config["TCP"]
                    .as_object()
                    .is_some_and(serde_json::Map::is_empty)
                    && lease.absent.get("TCP") == Some(&true)
                {
                    config
                        .as_object_mut()
                        .ok_or(Error::Contract("invalid config"))?
                        .remove("TCP");
                }
                if self.set(&config, &etag).await? {
                    break;
                }
            }
            let (config, _) = self.get().await?;
            if config["TCP"].get(lease.port.to_string()).is_some() {
                return Err(Error::Contract("Serve removal contention"));
            }
            std::fs::remove_file(self.home.join("remote/lease.json"))?;
            return Ok(());
        }

        for _ in 0..5 {
            let (mut config, etag) = self.get().await?;
            if config["Web"].get(&hp).is_none() {
                break;
            }
            if config["Web"][&hp] != json!({"Handlers":{"/":{"Proxy":lease.target}}})
                || config["TCP"][lease.port.to_string()] != json!({"HTTPS":true})
                || (!lease.tailnet && config["AllowFunnel"][&hp] != true)
            {
                return Err(Error::Contract("owned Funnel mapping changed"));
            }
            let mut owned = vec![("Web", hp.clone()), ("TCP", lease.port.to_string())];
            if !lease.tailnet {
                owned.push(("AllowFunnel", hp.clone()));
            }
            for (key, index) in owned {
                let map = config[key]
                    .as_object_mut()
                    .ok_or(Error::Contract("invalid Serve section"))?;
                map.remove(&index);
                if map.is_empty() && lease.absent.get(key) == Some(&true) {
                    config
                        .as_object_mut()
                        .ok_or(Error::Contract("invalid config"))?
                        .remove(key);
                }
            }
            if self.set(&config, &etag).await? {
                std::fs::remove_file(self.home.join("remote/lease.json"))?;
                return Ok(());
            }
        }
        let (config, _) = self.get().await?;
        if config["Web"].get(&hp).is_some() {
            return Err(Error::Contract("Serve removal contention"));
        }
        std::fs::remove_file(self.home.join("remote/lease.json"))?;
        Ok(())
    }
}
impl Ingress for Tailscale {
    fn run(&self, gateway: Gateway, mut stop: watch::Receiver<bool>) -> IngressFuture<'_> {
        Box::pin(async move {
            if !self.await_ready(&gateway, &mut stop).await? {
                return Ok(());
            }
            self.remove().await?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let path = format!("/ab-proxy/{}", state::token());
            let lease = self
                .add(&format!("http://{}{path}", listener.local_addr()?))
                .await?;
            let host = if lease.port == 443 {
                lease.host.clone()
            } else {
                format!("{}:{}", lease.host, lease.port)
            };
            let name = self.exposure.ingress();
            let tailnet_only = self.exposure == Exposure::Tailnet;
            let _active = gateway.ready(name, &host);
            let private = lease.private;
            let (probe_host, probe_port) = (lease.host.clone(), lease.port);
            let probe_gateway = gateway.clone();
            let probe = tokio::spawn(async move {
                if private {
                    return;
                }
                loop {
                    let wait =
                        match crate::localapi::certificate_expiry(&probe_host, probe_port).await {
                            Ok(expiry) => {
                                probe_gateway.certificate_expiry(name, &expiry);
                                std::time::Duration::from_secs(3600)
                            }
                            Err(_) => std::time::Duration::from_secs(15),
                        };
                    tokio::time::sleep(wait).await;
                }
            });
            let mut tasks = JoinSet::new();
            let result = loop {
                tokio::select! {
                    () = stopped(&mut stop) => break Ok(()),
                    result = listener.accept() => match result {
                        Ok((socket, addr)) if addr.ip().is_loopback() => {
                            let path = path.clone(); let host = host.clone(); let gateway = gateway.clone();
                            tasks.spawn(async move { proxy(socket, gateway, &host, &path, tailnet_only).await });
                        }
                        Ok(_) => {}
                        Err(e) => break Err(e.into()),
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            };
            probe.abort();
            tasks.abort_all();
            let cleanup = self.remove().await;
            result.and(cleanup)
        })
    }
}
#[must_use]
pub fn tailnet_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && (64..128).contains(&b)
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}
async fn proxy(
    mut socket: tokio::net::TcpStream,
    gateway: Gateway,
    host: &str,
    path: &str,
    tailnet_only: bool,
) -> Result<()> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 32768 {
            return Err(Error::Contract("proxy header limit"));
        }
        bytes.push(socket.read_u8().await?);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::Contract("invalid HTTP header"))?;
    let mut lines = text.split("\r\n");
    let mut request = lines
        .next()
        .ok_or(Error::Contract("missing request"))?
        .split(' ');
    let method = request.next().ok_or(Error::Contract("missing method"))?;
    let uri = request.next().ok_or(Error::Contract("missing URI"))?;
    let version = request.next().ok_or(Error::Contract("missing version"))?;
    let suffix = uri
        .strip_prefix(path)
        .filter(|v| v.is_empty() || v.starts_with('/') || v.starts_with('?'))
        .ok_or(Error::Contract("invalid proxy capability"))?;
    let mut headers = BTreeMap::<String, Vec<&str>>::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or(Error::Contract("invalid header"))?;
        headers
            .entry(key.to_ascii_lowercase())
            .or_default()
            .push(value.trim());
    }
    for (key, expected) in [
        ("host", host),
        ("x-forwarded-host", host),
        ("x-forwarded-proto", "https"),
    ] {
        if headers.get(key).map(Vec::as_slice) != Some(&[expected][..]) {
            return Err(Error::Contract("untrusted proxy headers"));
        }
    }
    let source = headers
        .get("x-forwarded-for")
        .filter(|v| v.len() == 1)
        .and_then(|v| v[0].parse::<std::net::IpAddr>().ok())
        .ok_or(Error::Contract("invalid proxy source"))?;
    if tailnet_only
        && (!tailnet_address(source) || headers.contains_key("tailscale-funnel-request"))
    {
        return Err(Error::Contract("tailnet route refused an outside source"));
    }
    let suffix = if suffix.is_empty() {
        "/".to_owned()
    } else if suffix.starts_with('?') {
        format!("/{suffix}")
    } else {
        suffix.into()
    };
    let mut output = format!("{method} {suffix} {version}\r\n");
    for (key, values) in headers {
        if key.starts_with("x-forwarded-") || key == "connection" {
            continue;
        }
        for value in values {
            output.push_str(&key);
            output.push_str(": ");
            output.push_str(value);
            output.push_str("\r\n");
        }
    }
    if text.to_ascii_lowercase().contains("upgrade: websocket\r\n") {
        output.push_str("connection: upgrade\r\n");
    } else {
        output.push_str("connection: close\r\n");
    }
    output.push_str("\r\n");
    let mut upstream = gateway.connect_stream(None, Some(source)).await?;
    upstream.write_all(output.as_bytes()).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    Ok(())
}
