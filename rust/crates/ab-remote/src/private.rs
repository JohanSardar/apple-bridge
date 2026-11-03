use crate::{Error, Gateway, Ingress, IngressFuture, Result, state, stopped, tailscale::Tailscale};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
    sync::{Mutex, watch},
    task::JoinSet,
};

pub fn socket_path() -> PathBuf {
    std::env::var_os("APPLE_BRIDGE_TAILSCALE_SOCKET")
        .map_or_else(|| "/var/run/tailscaled.socket".into(), PathBuf::from)
}
#[derive(Serialize, Deserialize)]
struct Invitation {
    host: String,
    secret: String,
}
#[derive(Debug)]
pub struct Private {
    pub home: PathBuf,
    pub tailscale: Tailscale,
}
struct Grant {
    secret: String,
    expires: Instant,
    session: Option<String>,
}
impl Grant {
    fn redeem(&mut self, authorization: &str) -> Option<String> {
        if self.session.is_some()
            || Instant::now() >= self.expires
            || !bool::from(
                authorization
                    .as_bytes()
                    .ct_eq(format!("Bearer {}", self.secret).as_bytes()),
            )
        {
            return None;
        }
        let session = state::token();
        self.session = Some(session.clone());
        Some(session)
    }
}
impl Ingress for Private {
    fn run(&self, gateway: Gateway, mut stop: watch::Receiver<bool>) -> IngressFuture<'_> {
        Box::pin(async move {
            self.tailscale.remove().await?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let lease = self
                .tailscale
                .add_mode(&listener.local_addr()?.to_string(), true)
                .await?;
            let ttl = Duration::from_secs(300);
            #[cfg(feature = "lab-harness")]
            let ttl = std::env::var("APPLE_BRIDGE_LAB_PRIVATE_TTL")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(ttl);
            let grant = Arc::new(Mutex::new(Grant {
                secret: state::token(),
                expires: Instant::now() + ttl,
                session: None,
            }));
            let code = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Invitation {
                host: lease.host,
                secret: grant.lock().await.secret.clone(),
            })?);
            state::write(&self.home, "invitation", &format!("bridge connect {code}"))?;
            let _active = gateway.ready("private", "localhost:7780");
            let mut tasks = JoinSet::new();
            let result: Result<()> = loop {
                tokio::select! {
                    () = stopped(&mut stop) => break Ok(()),
                    result = listener.accept(), if tasks.len() < 64 => {
                        let (socket, _) = match result { Ok(value) => value, Err(error) => break Err(error.into()) };
                        let gateway = gateway.clone(); let grant = grant.clone();
                        tasks.spawn(async move { serve(socket, gateway, grant).await });
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            };
            tasks.abort_all();
            let _ = std::fs::remove_file(self.home.join("remote/invitation.json"));
            result.and(self.tailscale.remove().await)
        })
    }
}
async fn head(socket: &mut (impl AsyncRead + Unpin)) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            if bytes.len() >= 32768 {
                return Err(Error::Contract("private header limit"));
            }
            bytes.push(socket.read_u8().await?);
        }
        String::from_utf8(bytes).map_err(|_| Error::Contract("invalid private header"))
    })
    .await
    .map_err(|_| Error::Contract("private header timeout"))?
}
fn header<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let values = text
        .split("\r\n")
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
        .collect::<Vec<_>>();
    if values.len() == 1 {
        Some(values[0])
    } else {
        None
    }
}
async fn reply(socket: &mut TcpStream, status: &str, body: &str) -> Result<()> {
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
    Ok(())
}
async fn serve(mut socket: TcpStream, gateway: Gateway, grant: Arc<Mutex<Grant>>) -> Result<()> {
    let text = head(&mut socket).await?;
    if header(&text, "host") != Some("localhost:7780") {
        return reply(&mut socket, "403 Forbidden", "{}").await;
    }
    let first = text
        .lines()
        .next()
        .ok_or(Error::Contract("missing request"))?;
    if first.split_whitespace().nth(1) == Some("/_bridge/private/invite") {
        let session = if first.starts_with("POST ") {
            grant
                .lock()
                .await
                .redeem(header(&text, "authorization").unwrap_or_default())
        } else {
            None
        };
        return match session {
            Some(session) => {
                reply(
                    &mut socket,
                    "200 OK",
                    &serde_json::json!({"session":session}).to_string(),
                )
                .await
            }
            None => reply(&mut socket, "403 Forbidden", "{}").await,
        };
    }
    let allowed = grant.lock().await.session.as_ref().is_some_and(|session| {
        header(&text, "x-bridge-private-session")
            .is_some_and(|received| bool::from(session.as_bytes().ct_eq(received.as_bytes())))
    });
    if !allowed {
        return reply(&mut socket, "403 Forbidden", "{}").await;
    }
    let output = rewrite(&text, None)?;
    let mut upstream = gateway.connect().await?;
    upstream.write_all(output.as_bytes()).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    Ok(())
}
fn rewrite(text: &str, session: Option<&str>) -> Result<String> {
    let mut lines = text.split("\r\n");
    let mut out = format!(
        "{}\r\n",
        lines.next().ok_or(Error::Contract("missing request"))?
    );
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, _) = line
            .split_once(':')
            .ok_or(Error::Contract("invalid header"))?;
        let key = key.to_ascii_lowercase();
        if key == "x-bridge-private-session"
            || key == "forwarded"
            || key.starts_with("x-forwarded-")
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    if let Some(session) = session {
        out.push_str("X-Bridge-Private-Session: ");
        out.push_str(session);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    Ok(out)
}
async fn dial(host: &str) -> Result<UnixStream> {
    let mut socket = UnixStream::connect(socket_path()).await?;
    socket.write_all(format!("POST /localapi/v0/dial HTTP/1.1\r\nHost: local-tailscaled.sock\r\nConnection: upgrade\r\nUpgrade: ts-dial\r\nDial-Host: {host}\r\nDial-Port: 7780\r\nDial-Network: tcp\r\nContent-Length: 0\r\n\r\n").as_bytes()).await?;
    if !head(&mut socket).await?.starts_with("HTTP/1.1 101 ") {
        return Err(Error::Contract("Tailscale private dial rejected"));
    }
    Ok(socket)
}
pub async fn connect(code: &str, address: &str) -> Result<()> {
    if code.len() > 4096 {
        return Err(Error::Contract("invalid private code"));
    }
    let raw = URL_SAFE_NO_PAD
        .decode(code)
        .map_err(|_| Error::Contract("invalid private code"))?;
    let invitation: Invitation = serde_json::from_slice(&raw)?;
    if !invitation.host.ends_with(".ts.net")
        || invitation
            .host
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && b != b'.' && b != b'-')
        || invitation.secret.len() != 43
        || URL_SAFE_NO_PAD.decode(&invitation.secret).is_err()
    {
        return Err(Error::Contract("invalid private host or secret"));
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(Error::Contract("private proxy requires loopback"));
    }
    let mut upstream = dial(&invitation.host).await?;
    upstream.write_all(format!("POST /_bridge/private/invite HTTP/1.1\r\nHost: localhost:7780\r\nAuthorization: Bearer {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",invitation.secret).as_bytes()).await?;
    let response = head(&mut upstream).await?;
    if !response.starts_with("HTTP/1.1 200 ") {
        return Err(Error::Contract("private invitation rejected"));
    }
    let length = header(&response, "content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n <= 4096)
        .ok_or(Error::Contract("invalid private grant"))?;
    let mut body = vec![0; length];
    upstream.read_exact(&mut body).await?;
    let grant: serde_json::Value = serde_json::from_slice(&body)?;
    let session = grant["session"]
        .as_str()
        .filter(|v| v.len() == 43 && URL_SAFE_NO_PAD.decode(v).is_ok())
        .ok_or(Error::Contract("invalid session"))?
        .to_owned();
    println!("http://localhost:7780");
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept(), if tasks.len() < 64 => {
                let (mut socket, _) = accepted?; let host = invitation.host.clone(); let session = session.clone();
                tasks.spawn(async move {
                    let text = head(&mut socket).await?;
                    if header(&text,"host") != Some("localhost:7780") { return reply(&mut socket,"403 Forbidden","{}").await; }
                    let mut upstream = dial(&host).await?;
                    upstream.write_all(rewrite(&text,Some(&session))?.as_bytes()).await?;
                    tokio::io::copy_bidirectional(&mut socket,&mut upstream).await?;
                    Ok(())
                });
            },
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            _ = tokio::signal::ctrl_c() => return Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitations_expire_and_are_single_use() {
        let mut grant = Grant {
            secret: "secret".into(),
            expires: Instant::now() + Duration::from_secs(300),
            session: None,
        };
        assert!(grant.redeem("Bearer wrong").is_none());
        assert!(grant.redeem("Bearer secret").is_some());
        assert!(grant.redeem("Bearer secret").is_none());
        grant.session = None;
        grant.expires = Instant::now();
        assert!(grant.redeem("Bearer secret").is_none());
    }
}
