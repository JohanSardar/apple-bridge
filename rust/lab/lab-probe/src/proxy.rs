use crate::{Error, Result};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    net::{IpAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Route {
    pub target: String,
    pub upstream: std::net::SocketAddr,
}
impl Route {
    pub fn parse(host: &str, ip: &str, allow_private: bool) -> Result<Self> {
        let rest = host
            .strip_prefix("https://")
            .ok_or(Error::Invalid("route"))?;
        let rest = rest.trim_end_matches('/');
        if rest.is_empty() || rest.contains(['/', '@', '?', '#']) {
            return Err(Error::Invalid("route"));
        }
        let (name, port) = match rest.rsplit_once(':') {
            Some((name, port)) => (
                name,
                port.parse::<u16>().map_err(|_| Error::Invalid("port"))?,
            ),
            None => (rest, 443),
        };
        let address: IpAddr = ip.parse().map_err(|_| Error::Invalid("route"))?;
        if !allow_private
            && (address.is_loopback() || is_private(address) || address.is_unspecified())
        {
            return Err(Error::Invalid("route"));
        }
        Ok(Self {
            target: format!("{name}:{port}"),
            upstream: std::net::SocketAddr::new(address, port),
        })
    }
}
fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}
fn publish(ready: &Path, url: &str, connections: usize) -> Result<()> {
    let temporary = ready.with_extension("tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&json!({"url": url, "connections": connections}))?,
    )?;
    std::fs::rename(temporary, ready)?;
    Ok(())
}
pub fn serve(
    listener: &TcpListener,
    route: &Route,
    ready: &Path,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let url = format!("http://{}", listener.local_addr()?);
    publish(ready, &url, 0)?;
    listener.set_nonblocking(true)?;
    let connections = Arc::new(Mutex::new(0usize));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((client, _)) => {
                let (route, ready, url, connections) = (
                    route.clone(),
                    ready.to_owned(),
                    url.clone(),
                    connections.clone(),
                );
                thread::spawn(move || {
                    let _ = tunnel(client, &route, &ready, &url, &connections);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn refuse(client: &mut TcpStream, line: &str) -> Result<()> {
    client.write_all(
        format!("HTTP/1.1 {line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    Ok(())
}
fn tunnel(
    mut client: TcpStream,
    route: &Route,
    ready: &Path,
    url: &str,
    connections: &Mutex<usize>,
) -> Result<()> {
    client.set_nonblocking(false)?;
    client.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    let mut parts = request.split_whitespace();
    if parts.next() != Some("CONNECT") || parts.next() != Some(route.target.as_str()) {
        return refuse(&mut client, "403 Forbidden");
    }
    let Ok(upstream) = TcpStream::connect_timeout(&route.upstream, Duration::from_secs(10)) else {
        return refuse(&mut client, "502 Bad Gateway");
    };
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    {
        let mut count = connections.lock().map_err(|_| Error::Invalid("lock"))?;
        *count += 1;
        publish(ready, url, *count)?;
    }
    client.set_read_timeout(None)?;
    let (mut up_read, mut up_write) = (upstream.try_clone()?, upstream);
    let mut down = client.try_clone()?;
    let forward = thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut up_write);
        let _ = up_write.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut up_read, &mut down);
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = forward.join();
    Ok(())
}
