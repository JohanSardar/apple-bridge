#![allow(clippy::unwrap_used, clippy::too_many_lines)]
use lab_probe::{Input, Route, checks, serve_proxy};
use lab_relay::Ca;
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn respond(status: u16, content_type: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    )
}
fn handle(stream: &mut StreamOwned<ServerConnection, TcpStream>) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader.read_line(&mut request).ok()?;
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line == "\r\n" {
            break;
        }
        let lower = line.to_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().ok()?;
        }
        headers.push(lower);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let body = String::from_utf8(body).ok()?;
    let has = |needle: &str| headers.iter().any(|h| h.contains(needle));
    let mut parts = request.split_whitespace();
    let (method, path) = (parts.next()?, parts.next()?);
    Some(match (method, path) {
        ("GET", "/") => respond(200, "text/html", "", "<div id=\"root\"></div>"),
        ("GET", "/v1/health") => respond(200, "application/json", "", "{}"),
        ("POST", "/v1/op/records.query") => {
            if has("origin:") || !has("bearer remote-token") {
                respond(401, "application/json", "", "{}")
            } else {
                respond(200, "application/json", "", "{\"records\":[]}")
            }
        }
        ("GET", "/v1/feed") => respond(426, "text/plain", "", ""),
        ("POST", "/auth/login/options") => respond(
            200,
            "application/json",
            "",
            "{\"publicKey\":{\"rpId\":\"localhost\"}}",
        ),
        ("POST", "/mcp") => {
            let sse = |value: &str| {
                respond(
                    200,
                    "text/event-stream",
                    "Mcp-Session-Id: s1\r\n",
                    &format!("event: message\ndata: {value}\n\n"),
                )
            };
            if body.contains("\"initialize\"") {
                respond(
                    200,
                    "application/json",
                    "Mcp-Session-Id: s1\r\n",
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
                )
            } else if body.contains("tools/list") {
                sse(
                    "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"records_query\"}]}}",
                )
            } else if body.contains("tools/call") {
                sse("{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"content\":[],\"isError\":false}}")
            } else {
                respond(202, "text/plain", "", "")
            }
        }
        _ => respond(404, "text/plain", "", ""),
    })
}
#[test]
fn remote_checks_pass_against_a_conforming_server() {
    let ca = Ca::new().unwrap();
    let (chain_pem, key_pem) = ca.leaf().unwrap();
    let chain = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let config = config.clone();
            thread::spawn(move || {
                tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), tcp);
                if let Some(reply) = handle(&mut stream) {
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.flush();
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                }
            });
        }
    });
    let input = Input {
        url: format!("https://localhost:{port}"),
        ip: "127.0.0.1".into(),
        remote: "remote-token".into(),
        local: "local-token".into(),
        off: false,
    };
    let result = checks(&input, ca.roots().unwrap()).unwrap();
    let all = result["checks"].as_object().unwrap();
    assert_eq!(all.len(), 12 + 20, "{result}");
    assert!(all.values().all(|v| v == true), "{result}");
    assert_eq!(result["latency"]["samples"], 20);
    let off = Input { off: true, ..input };
    let result = checks(&off, ca.roots().unwrap()).unwrap();
    assert_eq!(result["checks"]["off_public_stopped"], false);
}
#[test]
fn proxy_tunnels_only_the_pinned_target() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = upstream.local_addr().unwrap().port();
    thread::spawn(move || {
        if let Ok((mut socket, _)) = upstream.accept() {
            let mut buffer = [0; 5];
            if socket.read_exact(&mut buffer).is_ok() {
                let _ = socket.write_all(&buffer);
            }
        }
    });
    let directory = std::env::temp_dir().join(format!("lab-probe-{port}"));
    std::fs::create_dir_all(&directory).unwrap();
    let ready = directory.join("ready.json");
    let route = Route::parse(&format!("https://relay.example:{port}"), "127.0.0.1", true).unwrap();
    assert!(Route::parse("https://relay.example", "127.0.0.1", false).is_err());
    assert!(Route::parse("https://relay.example", "10.1.2.3", false).is_err());
    assert!(Route::parse("https://relay.example", "8.8.8.8", false).is_ok());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker = {
        let (stop, ready) = (stop.clone(), ready.clone());
        thread::spawn(move || serve_proxy(&listener, &route, &ready, &stop).unwrap())
    };
    let mut refused = TcpStream::connect(address).unwrap();
    refused
        .write_all(b"CONNECT other.example:1 HTTP/1.1\r\n\r\n")
        .unwrap();
    let mut text = String::new();
    BufReader::new(refused).read_line(&mut text).unwrap();
    assert!(text.contains("403"));
    let mut client = TcpStream::connect(address).unwrap();
    client
        .write_all(format!("CONNECT relay.example:{port} HTTP/1.1\r\n\r\n").as_bytes())
        .unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(status.contains("200"));
    let mut blank = String::new();
    reader.read_line(&mut blank).unwrap();
    client.write_all(b"hello").unwrap();
    let mut echoed = [0; 5];
    reader.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"hello");
    let published: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
    assert_eq!(published["connections"], 1);
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
