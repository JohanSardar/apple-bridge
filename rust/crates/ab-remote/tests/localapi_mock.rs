use ab_remote::{localapi::LocalApi, tailscale::Tailscale};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Mock {
    config: Value,
    etag: u32,
    seen: Vec<String>,
    conflict_once: bool,
}

async fn respond(
    socket: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    state: &Mutex<Mock>,
    token: &str,
) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let (head, mut body) = loop {
        let read = socket.read(&mut buffer).await.unwrap();
        if read == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break (
                String::from_utf8_lossy(&bytes[..at]).into_owned(),
                bytes[at + 4..].to_vec(),
            );
        }
    };
    let header = |name: &str| {
        head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    let length = header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    while body.len() < length {
        let read = socket.read(&mut buffer).await.unwrap();
        body.extend_from_slice(&buffer[..read]);
    }
    let mut first = head.lines().next().unwrap().split(' ');
    let (method, path) = (first.next().unwrap(), first.next().unwrap());
    let reply = {
        let mut mock = state.lock().unwrap();
        mock.seen.push(format!("{method} {path}"));
        let expected = format!(
            "Basic {}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!(":{token}")
            )
        );
        let (status, etag, payload) = if header("authorization").as_deref()
            != Some(expected.as_str())
            || header("host").as_deref() != Some("local-tailscaled.sock")
            || header("tailscale-cap").is_none()
        {
            ("401 Unauthorized", None, json!({}))
        } else if path == "/localapi/v0/status?peers=false" {
            (
                "200 OK",
                None,
                json!({"BackendState":"Running","Self":{"DNSName":"node.example.ts.net.","CapMap":{"https":null,"funnel":null,"https://tailscale.com/cap/funnel-ports?ports=443,8443,10000":null}}}),
            )
        } else if path == "/localapi/v0/serve-config" && method == "GET" {
            ("200 OK", Some(mock.etag), mock.config.clone())
        } else if path == "/localapi/v0/serve-config" && method == "POST" {
            if header("if-match") != Some(mock.etag.to_string()) {
                ("412 Precondition Failed", None, json!({}))
            } else if mock.conflict_once {
                mock.conflict_once = false;
                mock.etag += 1;
                mock.config["Unrelated"] = json!(true);
                ("412 Precondition Failed", None, json!({}))
            } else {
                mock.config = serde_json::from_slice(&body).unwrap();
                mock.etag += 1;
                ("200 OK", None, json!({}))
            }
        } else {
            ("404 Not Found", None, json!({}))
        };
        let payload = payload.to_string();
        let etag = etag.map_or_else(String::new, |etag| format!("ETag: {etag}\r\n"));
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        reply
    };
    socket.write_all(reply.as_bytes()).await.unwrap();
}

async fn serve(token: &'static str, config: Value, conflict_once: bool) -> (u16, Arc<Mutex<Mock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(Mutex::new(Mock {
        config,
        etag: 1,
        seen: vec![],
        conflict_once,
    }));
    let shared = state.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let state = shared.clone();
            tokio::spawn(async move { respond(&mut socket, &state, token).await });
        }
    });
    (port, state)
}

#[tokio::test]
async fn adds_and_removes_over_tcp_with_basic_auth() {
    let (port, state) = serve(
        "tok-123",
        json!({"Web":{"other.ts.net:443":{"Handlers":{}}}}),
        true,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let tailscale = Tailscale::tcp(home.path().to_owned(), port, "tok-123".into());
    let lease = tailscale.add("http://127.0.0.1:9/secret").await.unwrap();
    assert_eq!(
        (lease.host.as_str(), lease.port),
        ("node.example.ts.net", 443)
    );
    {
        let mock = state.lock().unwrap();
        assert_eq!(mock.config["Unrelated"], json!(true));
        assert_eq!(mock.config["TCP"]["443"], json!({"HTTPS":true}));
        assert_eq!(
            mock.config["Web"]["node.example.ts.net:443"],
            json!({"Handlers":{"/":{"Proxy":"http://127.0.0.1:9/secret"}}})
        );
        assert_eq!(
            mock.config["AllowFunnel"]["node.example.ts.net:443"],
            json!(true)
        );
        assert!(mock.config["Web"]["other.ts.net:443"].is_object());
    }
    let stored: Value =
        serde_json::from_slice(&std::fs::read(home.path().join("remote/lease.json")).unwrap())
            .unwrap();
    assert_eq!(stored["port"], 443);
    assert_eq!(
        stored["absent"],
        json!({"TCP":true,"Web":false,"AllowFunnel":true})
    );
    tailscale.remove().await.unwrap();
    let mock = state.lock().unwrap();
    assert_eq!(mock.config.get("TCP"), None);
    assert_eq!(mock.config.get("AllowFunnel"), None);
    assert!(mock.config["Web"]["other.ts.net:443"].is_object());
    assert!(mock.config["Web"].get("node.example.ts.net:443").is_none());
    assert!(!home.path().join("remote/lease.json").exists());
    assert!(
        mock.seen
            .iter()
            .any(|line| line == "GET /localapi/v0/status?peers=false")
    );
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let (port, _) = serve("right", json!({}), false).await;
    let home = tempfile::tempdir().unwrap();
    let tailscale = Tailscale::tcp(home.path().to_owned(), port, "wrong".into());
    assert!(tailscale.add("http://127.0.0.1:9/x").await.is_err());
    assert!(!home.path().join("remote/lease.json").exists());
}

#[tokio::test]
async fn takes_over_a_go_written_lease() {
    let target = "http://127.0.0.1:54043/abc";
    let host = "node.example.ts.net";
    let (port, state) = serve(
        "t",
        json!({"TCP":{"10000":{"HTTPS":true}},"Web":{format!("{host}:10000"):{"Handlers":{"/":{"Proxy":target}}}},"AllowFunnel":{format!("{host}:10000"):true}}),
        false,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("remote")).unwrap();
    std::fs::write(
        home.path().join("remote/lease.json"),
        json!({"host":host,"port":10000,"target":target,"absent":{"AllowFunnel":false,"TCP":false,"Web":false}}).to_string(),
    )
    .unwrap();
    let tailscale = Tailscale::tcp(home.path().to_owned(), port, "t".into());
    tailscale.remove().await.unwrap();
    assert_eq!(state.lock().unwrap().config["TCP"], json!({}));
    let lease = tailscale.add("http://127.0.0.1:55555/new").await.unwrap();
    assert_eq!(lease.port, 443);
}

#[tokio::test]
async fn unix_socket_path_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ts.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let read = socket.read(&mut buffer).await.unwrap();
        let text = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
        assert!(!text.contains("authorization"));
        assert!(text.contains("host: local-tailscaled.sock"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .unwrap();
    });
    let response = LocalApi::Unix(path)
        .request(reqwest::Method::GET, "status?peers=false", None, None)
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}
