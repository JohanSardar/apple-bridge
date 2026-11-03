use crate::{Error, Gateway, Ingress, IngressFuture, Result, state, stopped};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
    task::JoinSet,
};

#[derive(Debug)]
pub struct Tailscale {
    pub home: PathBuf,
    pub client: reqwest::Client,
    pub endpoint: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Lease {
    pub host: String,
    pub port: u16,
    pub target: String,
    pub absent: BTreeMap<String, bool>,
    #[serde(default)]
    pub private: bool,
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
            client: reqwest::Client::builder()
                .unix_socket(socket)
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            endpoint: "http://local-tailscaled.sock/localapi/v0".into(),
        })
    }
    async fn get(&self) -> Result<(Value, String)> {
        let response = self
            .client
            .get(format!("{}/serve-config", self.endpoint))
            .send()
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
            .client
            .post(format!("{}/serve-config", self.endpoint))
            .header("If-Match", etag)
            .json(value)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Ok(false);
        }
        response.error_for_status()?;
        Ok(true)
    }
    pub async fn add(&self, target: &str) -> Result<Lease> {
        self.add_mode(target, false).await
    }
    pub async fn add_mode(&self, target: &str, private: bool) -> Result<Lease> {
        let status: Value = self
            .client
            .get(format!("{}/status?peers=false", self.endpoint))
            .send()
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
        if !private && (!has("https") || !has("funnel")) {
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
            } else {
                vec![443_u16, 8443, 10000]
            } {
                let permitted = allowed.iter().any(|list| list.split(',').any(|range| {
                    if let Some((a,b)) = range.split_once('-') { matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a),Ok(b)) if (a..=b).contains(&port)) } else { range.parse::<u16>() == Ok(port) }
                }));
                if (private || permitted) && !port_used(&config, port)? {
                    selected = Some(port);
                    break;
                }
            }
            let port = selected.ok_or(Error::Contract("no free allowed Funnel port"))?;
            let hp = format!("{host}:{port}");
            let mut lease = Lease {
                host: host.into(),
                port,
                target: target.into(),
                absent: BTreeMap::new(),
                private,
            };
            let mappings = if private {
                vec![("TCP", port.to_string(), json!({"TCPForward":target}))]
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
                || config["AllowFunnel"][&hp] != true
            {
                return Err(Error::Contract("owned Funnel mapping changed"));
            }
            for (key, index) in [
                ("Web", hp.clone()),
                ("TCP", lease.port.to_string()),
                ("AllowFunnel", hp.clone()),
            ] {
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
            let _active = gateway.ready("system", &host);
            let mut tasks = JoinSet::new();
            let result = loop {
                tokio::select! {
                    () = stopped(&mut stop) => break Ok(()),
                    result = listener.accept() => match result {
                        Ok((socket, addr)) if addr.ip().is_loopback() => {
                            let path = path.clone(); let host = host.clone(); let gateway = gateway.clone();
                            tasks.spawn(async move { proxy(socket, gateway, &host, &path).await });
                        }
                        Ok(_) => {}
                        Err(e) => break Err(e.into()),
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            };
            tasks.abort_all();
            let cleanup = self.remove().await;
            result.and(cleanup)
        })
    }
}
async fn proxy(
    mut socket: tokio::net::TcpStream,
    gateway: Gateway,
    host: &str,
    path: &str,
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
    let _ = source;
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
    let mut upstream = gateway.connect().await?;
    upstream.write_all(output.as_bytes()).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    Ok(())
}
