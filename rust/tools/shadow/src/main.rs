use ab_trust::{Authority, Grant, Principal, iphone};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    os::unix::fs::OpenOptionsExt,
    process::ExitCode,
    sync::Arc,
    time::Duration,
};
use x25519_dalek::{PublicKey, StaticSecret};

type Failure = Box<dyn std::error::Error>;

fn flags(args: impl Iterator<Item = String>) -> Result<BTreeMap<String, String>, Failure> {
    let mut args = args;
    let mut values = BTreeMap::new();
    while let Some(key) = args.next() {
        let value = args.next().ok_or("flag value required")?;
        values.insert(key, value);
    }
    Ok(values)
}
fn required<'a>(values: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, Failure> {
    Ok(values
        .get(key)
        .ok_or_else(|| format!("{key} required"))?
        .as_str())
}
fn random32() -> [u8; 32] {
    let mut bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}
fn iphone_device(args: impl Iterator<Item = String>) -> Result<(), Failure> {
    let values = flags(args)?;
    let home = required(&values, "--home")?;
    let output = required(&values, "--output")?;
    let authority = Authority::open(home)?;
    let raw = authority.pair_iphone_code(
        "https://iphone.invalid",
        false,
        Duration::from_secs(60),
    )?;
    let encoded = raw.strip_prefix("ab1-").ok_or("pair code prefix")?;
    let code: Value = serde_json::from_slice(
        &data_encoding::BASE32_NOPAD.decode(encoded.as_bytes())?,
    )?;
    let id = code["id"].as_str().ok_or("pair code id")?;
    let secret = STANDARD.decode(code["secret"].as_str().ok_or("pair code secret")?)?;
    let seed = random32();
    let signing = SigningKey::from_bytes(&seed);
    let public = signing.verifying_key().to_bytes();
    let encryption = PublicKey::from(&StaticSecret::from(random32()));
    let message = iphone::pair_message(id, &public, encryption.as_bytes());
    let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(&secret))?;
    mac.update(&message);
    let device = authority.pair_iphone(&iphone::PairRequest {
        id: id.to_owned(),
        signing_key: STANDARD.encode(public),
        encryption_key: STANDARD.encode(encryption.as_bytes()),
        proof: STANDARD.encode(mac.finalize().into_bytes()),
        signature: STANDARD.encode(signing.sign(&message).to_bytes()),
    })?;
    let record = serde_json::to_vec(&json!({
        "device_id": device.id,
        "signing_seed": STANDARD.encode(seed),
    }))?;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(output)?
        .write_all(&record)?;
    Ok(())
}
fn chunked(mut data: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut body = vec![];
    loop {
        let line = data
            .windows(2)
            .position(|pair| pair == b"\r\n")
            .ok_or("chunk size line")?;
        let size = usize::from_str_radix(
            std::str::from_utf8(&data[..line])?
                .split(';')
                .next()
                .unwrap_or_default()
                .trim(),
            16,
        )?;
        data = &data[line + 2..];
        if size == 0 {
            return Ok(body);
        }
        if data.len() < size + 2 {
            return Err("short chunk".into());
        }
        body.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}
fn post(
    authority: &Authority,
    fingerprint: &str,
    address: &str,
    header: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>), Failure> {
    let socket = address
        .to_socket_addrs()?
        .next()
        .ok_or("gateway address")?;
    let tcp = TcpStream::connect_timeout(&socket, Duration::from_secs(2))?;
    tcp.set_read_timeout(Some(Duration::from_secs(60)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(60)))?;
    let name = rustls::pki_types::ServerName::IpAddress(socket.ip().into());
    let connection = rustls::ClientConnection::new(
        Arc::new(authority.client_tls(fingerprint)?),
        name,
    )?;
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    let head = format!(
        "POST /v1/commands HTTP/1.1\r\nHost: {address}\r\nX-Apple-Bridge-Delegation: {header}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut raw = vec![];
    let mut buffer = [0; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => raw.extend_from_slice(&buffer[..count]),
            Err(error) if !raw.is_empty() && error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("response head")?;
    let head = std::str::from_utf8(&raw[..split])?;
    let status = head
        .split_whitespace()
        .nth(1)
        .ok_or("status line")?
        .parse::<u16>()?;
    let is_chunked = head
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding: chunked"));
    let payload = &raw[split + 4..];
    Ok((
        status,
        if is_chunked {
            chunked(payload)?
        } else {
            payload.to_vec()
        },
    ))
}
fn shadow_feed(args: impl Iterator<Item = String>) -> Result<(), Failure> {
    let values = flags(args)?;
    let peer_home = required(&values, "--peer-home")?;
    let address = required(&values, "--address")?;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let request: Value = serde_json::from_str(&input)?;
    let authority = Authority::open(peer_home)?;
    let enrollment = authority.enrollment()?.ok_or("peer home is not enrolled")?;
    let body = STANDARD.decode(request["body"].as_str().filter(|b| !b.is_empty()).ok_or("body required")?)?;
    let delegation = &request["delegation"];
    let text = |key: &str| delegation[key].as_str().unwrap_or_default().to_owned();
    let id = text("command_id");
    let hash = Some(text("request_hash"))
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| hex::encode(Sha256::digest(&body)));
    let grants = delegation["local_grants"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|grant| Grant::parse(grant.as_str().unwrap_or_default()))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let level = text("local_level");
    let principal = Principal {
        id: text("local_principal"),
        kind: if level.is_empty() { String::new() } else { "agent".into() },
        grants,
        level,
        ..Principal::default()
    };
    let header = authority.sign_delegation(&principal, &id, &hash, "")?;
    let (status, raw) = post(&authority, &enrollment.fingerprint, address, &header, &body)?;
    let mut outcome = json!({"status": 200});
    if status != 200 {
        let envelope: Value = serde_json::from_slice(&raw).map_err(|_| "gateway unavailable")?;
        let code = envelope["error"]["code"].as_str().ok_or("gateway unavailable")?;
        outcome = json!({"status": 400, "error": code});
    } else if request["method"] == "list" {
        let rows: Vec<Value> = serde_json::from_slice(&raw)?;
        outcome["count"] = rows.len().into();
    } else {
        let record: Value = serde_json::from_slice(&raw)?;
        outcome["state"] = record["state"].clone();
        outcome["disposition"] = record["disposition"].clone();
    }
    println!(
        "{}",
        json!({"outcome": outcome, "request_hash": hash, "command_id": id})
    );
    Ok(())
}
fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("iphone-device") => iphone_device(args),
        Some("shadow-feed") => shadow_feed(args),
        _ => Err("usage: ab-shadow-tools iphone-device --home DIR --output FILE | ab-shadow-tools shadow-feed --peer-home DIR --address HOST:PORT".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
