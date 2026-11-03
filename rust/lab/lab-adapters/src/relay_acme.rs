#![allow(clippy::too_many_lines)]
use crate::{
    phone::random,
    relay::{
        ADMIN, Fixture, Hooks, Lab, Limits, Tunnel, Wire, b64, cred, enroll, fixture_binary,
        http_with, post, signing_key,
    },
    relay_streams::{Identity, exchange, origin, proof_json},
    relay_tls::{client_config, connect},
};
use ed25519_dalek::SigningKey;
use lab_relay::Ca;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use p256::ecdsa::{Signature, SigningKey as AccountKey, signature::Signer};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType};
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, TcpListener, UdpSocket},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 2] = ["RELAY-ACME", "RELAY-OWN-DOMAIN"];
const DOMAIN: &str = "mac.example.test";

fn invalid<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
struct AcceptAny;
impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn insecure_config() -> Result<Arc<ClientConfig>> {
    Ok(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(invalid)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_no_client_auth(),
    ))
}

struct Dns {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl std::fmt::Debug for Dns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dns").finish_non_exhaustive()
    }
}
impl Drop for Dns {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn question(packet: &[u8]) -> Option<(String, u16, usize)> {
    let mut at = 12;
    let mut labels = vec![];
    loop {
        let length = usize::from(*packet.get(at)?);
        if length == 0 {
            break;
        }
        labels.push(
            String::from_utf8_lossy(packet.get(at + 1..at + 1 + length)?).to_ascii_lowercase(),
        );
        at += 1 + length;
    }
    let kind = u16::from_be_bytes([*packet.get(at + 1)?, *packet.get(at + 2)?]);
    Some((labels.join("."), kind, at + 5))
}

fn dns_server(hooks: Arc<Hooks>, alias: Option<(String, String)>) -> Result<Dns> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    let address = socket.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = thread::spawn(move || {
        let mut packet = [0u8; 4096];
        while !flag.load(Ordering::SeqCst) {
            let Ok((n, peer)) = socket.recv_from(&mut packet) else {
                continue;
            };
            let Some((name, kind, end)) = question(&packet[..n]) else {
                continue;
            };
            let mut reply = packet[..end].to_vec();
            reply[2] = 0x84 | (packet[2] & 1);
            reply[3] = 0;
            reply[6..12].fill(0);
            let lookup = match &alias {
                Some((custom, target)) if name == format!("_acme-challenge.{custom}") => {
                    format!("_acme-challenge.{target}")
                }
                _ => name,
            };
            let value = lock(&hooks.dns).get(&lookup).cloned().unwrap_or_default();
            if kind == 16 && !value.is_empty() {
                reply[7] = 1;
                reply.extend_from_slice(&[0xC0, 0x0C, 0, 16, 0, 1, 0, 0, 0, 1]);
                let text = value.as_bytes();
                reply.extend_from_slice(&u16::try_from(text.len() + 1).unwrap_or(0).to_be_bytes());
                reply.push(u8::try_from(text.len()).unwrap_or(0));
                reply.extend_from_slice(text);
            }
            let _ = socket.send_to(&reply, peer);
        }
    });
    Ok(Dns {
        address,
        stop,
        thread: Some(thread),
    })
}

struct Pebble {
    child: Child,
    address: String,
    root: PathBuf,
    config: Arc<ClientConfig>,
}
impl std::fmt::Debug for Pebble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pebble").finish_non_exhaustive()
    }
}
impl Drop for Pebble {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_pebble(directory: &Path, dns: &str) -> Result<Pebble> {
    let binary = std::env::var_os("AB_LAB_PEBBLE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Invalid("AB_LAB_PEBBLE is required for Pebble scenarios".into()))?;
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.to_string()
    };
    let authority_key = KeyPair::generate().map_err(invalid)?;
    let mut authority = CertificateParams::new(Vec::<String>::new()).map_err(invalid)?;
    authority.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    authority
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Pebble lab");
    let root = authority.self_signed(&authority_key).map_err(invalid)?;
    let issuer = Issuer::new(authority, authority_key);
    let server_key = KeyPair::generate().map_err(invalid)?;
    let mut server = CertificateParams::new(Vec::<String>::new()).map_err(invalid)?;
    server.subject_alt_names = vec![SanType::IpAddress(IpAddr::from([127, 0, 0, 1]))];
    server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let server_cert = server.signed_by(&server_key, &issuer).map_err(invalid)?;
    let root_path = directory.join("root.pem");
    let cert_path = directory.join("server.pem");
    let key_path = directory.join("server-key.pem");
    for (path, text) in [
        (&root_path, root.pem()),
        (&cert_path, server_cert.pem()),
        (&key_path, server_key.serialize_pem()),
    ] {
        fs::write(path, text)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let config_path = directory.join("pebble.json");
    fs::write(
        &config_path,
        json!({"pebble": {
            "listenAddress": address,
            "managementListenAddress": "127.0.0.1:0",
            "certificate": cert_path,
            "privateKey": key_path,
            "httpPort": 5002,
            "tlsPort": 5001,
            "profiles": {"default": {"description": "lab renewal", "validityPeriod": 86400}},
            "retryAfter": {"authz": 1, "order": 1}
        }})
        .to_string(),
    )?;
    let log = fs::File::create(directory.join("pebble.log"))?;
    let child = Command::new(binary)
        .args(["-config"])
        .arg(&config_path)
        .args(["-dnsserver", dns])
        .env("PEBBLE_VA_NOSLEEP", "1")
        .env("PEBBLE_WFE_NONCEREJECT", "0")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let config = client_config(&[root.der().clone()])?;
    let mut pebble = Pebble {
        child,
        address,
        root: root_path,
        config,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if https(&pebble.config, &pebble.address, "GET", "/dir", &[], b"")
            .is_ok_and(|r| r.status == 200)
        {
            return Ok(pebble);
        }
        if Instant::now() >= deadline {
            let _ = pebble.child.kill();
            return Err(Error::Invalid("Pebble did not start".into()));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Debug)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Answer {
    fn header(&self, name: &str) -> String {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(line) = rest.windows(2).position(|w| w == b"\r\n") {
        let Ok(size) = usize::from_str_radix(String::from_utf8_lossy(&rest[..line]).trim(), 16)
        else {
            break;
        };
        rest = &rest[line + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
    out
}

fn https(
    config: &Arc<ClientConfig>,
    address: &str,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> Result<Answer> {
    let mut session = connect(config, address, "127.0.0.1", 30)?;
    let extra = headers
        .iter()
        .fold(String::new(), |mut out, (name, value)| {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{name}: {value}\r\n"));
            out
        });
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n{extra}Content-Length: {}\r\n\r\n",
        body.len()
    );
    session.write_all(head.as_bytes())?;
    session.write_all(body)?;
    session.flush()?;
    let mut raw = vec![];
    let mut buffer = [0u8; 8192];
    loop {
        match session.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => raw.extend_from_slice(&buffer[..n]),
        }
    }
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Error::Invalid("incomplete reply".into()))?;
    let text = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = text.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Error::Invalid("status".into()))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        dechunk(&raw[end + 4..])
    } else {
        raw[end + 4..].to_vec()
    };
    Ok(Answer {
        status,
        headers,
        body,
    })
}

struct Acme {
    config: Arc<ClientConfig>,
    address: String,
    key: AccountKey,
    kid: String,
    nonce: String,
    directory: Value,
}
impl Acme {
    fn new(pebble: &Pebble) -> Result<Self> {
        let key = AccountKey::from_slice(&random::<32>()?).map_err(invalid)?;
        let directory = https(&pebble.config, &pebble.address, "GET", "/dir", &[], b"")?.json();
        Ok(Self {
            config: pebble.config.clone(),
            address: pebble.address.clone(),
            key,
            kid: String::new(),
            nonce: String::new(),
            directory,
        })
    }
    fn path(&self, url: &str) -> String {
        url.split_once(&self.address)
            .map_or_else(|| url.to_owned(), |(_, path)| path.to_owned())
    }
    fn jwk(&self) -> Result<Value> {
        let point = self.key.verifying_key().to_sec1_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            return Err(Error::Invalid("account key".into()));
        };
        Ok(json!({"crv": "P-256", "kty": "EC", "x": b64(x), "y": b64(y)}))
    }
    fn thumbprint(&self) -> Result<String> {
        let jwk = self.jwk()?;
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            jwk["x"].as_str().unwrap_or_default(),
            jwk["y"].as_str().unwrap_or_default()
        );
        Ok(b64(&Sha256::digest(canonical.as_bytes())))
    }
    fn fresh_nonce(&mut self) -> Result<()> {
        let url = self.directory["newNonce"].as_str().unwrap_or_default();
        let reply = https(
            &self.config,
            &self.address,
            "HEAD",
            &self.path(url),
            &[],
            b"",
        )?;
        self.nonce = reply.header("replay-nonce");
        Ok(())
    }
    fn post(&mut self, url: &str, payload: Option<&Value>) -> Result<Answer> {
        for _ in 0..3 {
            if self.nonce.is_empty() {
                self.fresh_nonce()?;
            }
            let mut protected = json!({"alg": "ES256", "nonce": self.nonce, "url": url});
            if self.kid.is_empty() {
                protected["jwk"] = self.jwk()?;
            } else {
                protected["kid"] = json!(self.kid);
            }
            let protected = b64(protected.to_string().as_bytes());
            let payload =
                payload.map_or_else(String::new, |value| b64(value.to_string().as_bytes()));
            let signature: Signature = self.key.sign(format!("{protected}.{payload}").as_bytes());
            let body = json!({
                "protected": protected,
                "payload": payload,
                "signature": b64(&signature.to_bytes()),
            });
            let reply = https(
                &self.config,
                &self.address,
                "POST",
                &self.path(url),
                &[("Content-Type", "application/jose+json".into())],
                body.to_string().as_bytes(),
            )?;
            self.nonce = reply.header("replay-nonce");
            if reply.json()["type"].as_str() == Some("urn:ietf:params:acme:error:badNonce") {
                self.nonce.clear();
                continue;
            }
            return Ok(reply);
        }
        Err(Error::Invalid("ACME nonce".into()))
    }
    fn poll(&mut self, url: &str, want: &str) -> Result<Value> {
        for _ in 0..80 {
            let value = self.post(url, None)?.json();
            let status = value["status"].as_str().unwrap_or_default();
            if status == want {
                return Ok(value);
            }
            if status == "invalid" {
                return Err(Error::Invalid(format!("ACME object invalid: {value}")));
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(Error::Invalid(format!("ACME object never became {want}")))
    }
    fn account(&mut self) -> Result<()> {
        let url = self.directory["newAccount"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let reply = self.post(&url, Some(&json!({"termsOfServiceAgreed": true})))?;
        if !(200..300).contains(&reply.status) {
            return Err(Error::Invalid("ACME account".into()));
        }
        self.kid = reply.header("location");
        Ok(())
    }
}

fn provision(
    acme: &mut Acme,
    home: &Path,
    host: &str,
    api: &str,
    key: &SigningKey,
    device: &Identity,
) -> Result<()> {
    let publish = |value: &str| -> bool {
        proof_json(api, key, device, &format!("dns:{value}")).is_some_and(|proof| {
            post(api, "/dns", &json!({"proof": proof, "value": value}), None).0 == 204
        })
    };
    if acme.kid.is_empty() {
        acme.account()?;
    }
    let url = acme.directory["newOrder"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let reply = acme.post(
        &url,
        Some(&json!({"identifiers": [{"type": "dns", "value": host}]})),
    )?;
    let order_url = reply.header("location");
    let order = reply.json();
    let outcome = (|| -> Result<()> {
        for authorization in order["authorizations"].as_array().into_iter().flatten() {
            let url = authorization.as_str().unwrap_or_default().to_owned();
            let object = acme.post(&url, None)?.json();
            if object["status"] == "valid" {
                continue;
            }
            let challenge = object["challenges"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|c| c["type"] == "dns-01")
                .ok_or_else(|| Error::Invalid("no dns-01 challenge".into()))?;
            let authorization_text = format!(
                "{}.{}",
                challenge["token"].as_str().unwrap_or_default(),
                acme.thumbprint()?
            );
            let value = b64(&Sha256::digest(authorization_text.as_bytes()));
            if !publish(&value) {
                return Err(Error::Invalid("relay refused the DNS challenge".into()));
            }
            let challenge_url = challenge["url"].as_str().unwrap_or_default().to_owned();
            acme.post(&challenge_url, Some(&json!({})))?;
            acme.poll(&url, "valid")?;
        }
        acme.poll(&order_url, "ready")?;
        let key_pair = KeyPair::generate().map_err(invalid)?;
        let request = CertificateParams::new(vec![host.to_owned()])
            .map_err(invalid)?
            .serialize_request(&key_pair)
            .map_err(invalid)?;
        let finalize = order["finalize"].as_str().unwrap_or_default().to_owned();
        acme.post(&finalize, Some(&json!({"csr": b64(request.der())})))?;
        let done = acme.poll(&order_url, "valid")?;
        let certificate = done["certificate"].as_str().unwrap_or_default().to_owned();
        let chain = acme.post(&certificate, None)?;
        if chain.status != 200 {
            return Err(Error::Invalid("certificate download".into()));
        }
        let directory = home.join("remote");
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        for (name, bytes) in [
            ("relay-cert.pem", chain.body.clone()),
            ("relay-key.pem", key_pair.serialize_pem().into_bytes()),
        ] {
            fs::write(directory.join(name), bytes)?;
            fs::set_permissions(directory.join(name), fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();
    let _ = publish("");
    outcome
}

fn leaf(home: &Path) -> Option<Vec<u8>> {
    use rustls::pki_types::pem::PemObject;
    let pem = fs::read(home.join("remote").join("relay-cert.pem")).ok()?;
    CertificateDer::pem_slice_iter(&pem)
        .next()?
        .ok()
        .map(|der| der.to_vec())
}
fn names_host(der: &[u8], host: &str) -> bool {
    x509_parser::parse_x509_certificate(der).is_ok_and(|(_, cert)| {
        cert.subject_alternative_name().ok().flatten().is_some_and(|san| {
            san.value.general_names.iter().any(|name| {
                matches!(name, x509_parser::extensions::GeneralName::DNSName(n) if n.eq_ignore_ascii_case(host))
            })
        })
    })
}
fn serial(der: &[u8]) -> Vec<u8> {
    x509_parser::parse_x509_certificate(der)
        .map(|(_, cert)| cert.raw_serial().to_vec())
        .unwrap_or_default()
}

fn pebble_stage(
    directory: &Path,
    lab: &Lab,
    key: &SigningKey,
    device: &Identity,
    custom: bool,
    e: &mut Evidence,
) -> Result<()> {
    let hooks = lab.hooks();
    let alias = custom.then(|| (DOMAIN.to_owned(), device.host.clone()));
    let dns = dns_server(hooks.clone(), alias)?;
    let pebble = start_pebble(directory, &dns.address)?;
    let home = directory.join("bridge");
    fs::create_dir_all(home.join("remote"))?;
    let host = if custom { DOMAIN } else { device.host.as_str() };
    let mut acme = Acme::new(&pebble)?;
    provision(&mut acme, &home, host, &lab.api, key, device)?;
    let first = leaf(&home).ok_or_else(|| Error::Invalid("certificate missing".into()))?;
    e.check("pebble_certificate_hostname", names_host(&first, host), "");
    provision(&mut acme, &home, host, &lab.api, key, device)
        .map_err(|error| Error::Invalid(format!("renew: {error}")))?;
    let second = leaf(&home).ok_or_else(|| Error::Invalid("certificate missing".into()))?;
    e.check(
        "renewal_inside_30_days",
        serial(&first) != serial(&second),
        "",
    );
    let private = fs::metadata(home.join("remote").join("relay-key.pem"))
        .is_ok_and(|m| m.permissions().mode() & 0o777 == 0o600);
    e.check("private_key_only_under_bridge_home", private, "");
    let cleaned = lock(&hooks.dns)
        .get(&format!("_acme-challenge.{}", device.host))
        .is_none_or(String::is_empty);
    e.check("dns_challenge_cleaned_up", cleaned, "");
    if !custom {
        let control = directory.join("control");
        let issued = lab
            .control(
                &format!("https://{}/dir", pebble.address),
                &pebble.root,
                &control,
                host,
            )
            .map_err(|error| Error::Invalid(format!("control: {error}")));
        e.check("control_dns01_certificate_issued", issued.is_ok(), "");
        issued?;
        let pem = fs::read(control.join("relay-cert.pem")).ok();
        e.check("control_certificate_available", pem.is_some(), "");
        let parsed = pem.and_then(|pem| {
            use rustls::pki_types::pem::PemObject;
            CertificateDer::pem_slice_iter(&pem)
                .next()?
                .ok()
                .map(|d| d.to_vec())
        });
        e.check(
            "control_certificate_hostname",
            parsed.is_some_and(|der| names_host(&der, host)),
            "",
        );
    }
    Ok(())
}

struct Rig {
    directory: PathBuf,
    fixture: Arc<Fixture>,
    lab: Lab,
    first: SigningKey,
    a: Value,
    b: Value,
}

fn prologue(world: &World, e: &mut Evidence, name: &str) -> Result<Option<Rig>> {
    let directory = world.directory.join(name);
    fs::create_dir_all(&directory)?;
    let binary = fixture_binary()?;
    let fixture = Arc::new(Fixture::open(
        &binary,
        &directory.join("relay.json"),
        Limits::standard(),
    )?);
    let lab = Lab::start_with(&fixture, ADMIN)?;
    let (first, second) = (signing_key()?, signing_key()?);
    let (a, code1) = enroll(&lab.api, &first, "");
    let (b, code2) = enroll(&lab.api, &second, "");
    e.check(
        "two_distinct_device_enrollments",
        code1 == 200 && code2 == 200 && cred(&a, "hostname") != cred(&b, "hostname"),
        "",
    );
    if code1 != 200 || code2 != 200 {
        return Ok(None);
    }
    Ok(Some(Rig {
        directory,
        fixture,
        lab,
        first,
        a,
        b,
    }))
}

fn device(value: &Value) -> Identity {
    Identity {
        host: cred(value, "hostname"),
        credential: cred(value, "credential"),
    }
}

fn dns_proofs(rig: &Rig, e: &mut Evidence) {
    let api = &rig.lab.api;
    let hooks = rig.lab.hooks();
    let (a, b) = (device(&rig.a), device(&rig.b));
    let Some(proof) = proof_json(api, &rig.first, &a, "dns:fixture") else {
        e.check("proof", false, "");
        return;
    };
    let mut input = json!({"proof": proof, "value": "fixture"});
    let first = post(api, "/dns", &input, None).0;
    let last = lock(&hooks.dns_last).clone();
    e.check(
        "signed_dns_own_host_only",
        first == 204 && last.0 == format!("_acme-challenge.{}", a.host) && last.1 == "fixture",
        "",
    );
    e.check(
        "dns_proof_replay_refused",
        post(api, "/dns", &input, None).0 == 403,
        "",
    );
    let calls = || hooks.dns_calls.load(Ordering::SeqCst);
    if let Some(mut proof) = proof_json(api, &rig.first, &a, "dns:fixture") {
        proof["hostname"] = json!(b.host);
        input["proof"] = proof;
    }
    e.check(
        "cross_device_dns_refused",
        post(api, "/dns", &input, None).0 == 403 && calls() == 1,
        "",
    );
    if let Some(proof) = proof_json(api, &rig.first, &a, "dns:fixture") {
        input["proof"] = proof;
    }
    input["value"] = json!("changed");
    e.check(
        "dns_value_bound_to_signature",
        post(api, "/dns", &input, None).0 == 403 && calls() == 1,
        "",
    );
}

fn missing_records(hooks: &Hooks, host: &str) -> Vec<String> {
    let mut missing = vec![];
    for (name, target) in [
        (DOMAIN.to_owned(), host.to_owned()),
        (
            format!("_acme-challenge.{DOMAIN}"),
            format!("_acme-challenge.{host}"),
        ),
    ] {
        let found = lock(&hooks.cnames)
            .get(&name)
            .is_some_and(|value| value.trim_end_matches('.').eq_ignore_ascii_case(&target));
        if !found {
            missing.push(format!("{name} CNAME {target}."));
        }
    }
    missing
}

fn own_domain(rig: &Rig, e: &mut Evidence) -> Result<()> {
    let api = &rig.lab.api;
    let hooks = rig.lab.hooks();
    let (a, b) = (device(&rig.a), device(&rig.b));
    let missing = missing_records(&hooks, &a.host);
    e.check(
        "exact_missing_cnames",
        missing
            == [
                format!("{DOMAIN} CNAME {}.", a.host),
                format!("_acme-challenge.{DOMAIN} CNAME _acme-challenge.{}.", a.host),
            ],
        "",
    );
    let claim = |who: &Identity| -> Option<Value> {
        Some(json!({
            "proof": proof_json(api, &rig.first, who, &format!("domain:{DOMAIN}"))?,
            "domain": DOMAIN
        }))
    };
    let reply = claim(&a).map(|input| post(api, "/domain", &input, None));
    e.check(
        "domain_endpoint_requires_records",
        reply.is_some_and(|(status, value)| {
            status == 200 && value["missing"].as_array().is_some_and(|m| m.len() == 2)
        }),
        "",
    );
    let forged = claim(&a).map(|mut input| {
        input["proof"]["hostname"] = json!(b.host);
        post(api, "/domain", &input, None).0
    });
    e.check(
        "unauthorized_domain_claim_rejected",
        forged == Some(403),
        "",
    );
    {
        let mut cnames = lock(&hooks.cnames);
        cnames.insert(DOMAIN.to_owned(), format!("{}.", a.host));
        cnames.insert(
            format!("_acme-challenge.{DOMAIN}"),
            format!("_acme-challenge.{}.", a.host),
        );
    }
    e.check(
        "own_domain_records_ready",
        missing_records(&hooks, &a.host).is_empty(),
        "",
    );
    let signed = claim(&a);
    e.check(
        "signed_domain_registration",
        signed
            .as_ref()
            .is_some_and(|input| post(api, "/domain", input, None).0 == 204),
        "",
    );
    e.check(
        "domain_registration_replay_rejected",
        signed.is_some_and(|input| post(api, "/domain", &input, None).0 == 403),
        "",
    );
    let ca = Ca::new().map_err(invalid)?;
    let served = origin(&ca, DOMAIN)?;
    let attached = rig
        .lab
        .attach_kept(
            &a.host,
            &a.credential,
            Tunnel::Dial(served.address.clone()),
            &Wire::new(&[]),
        )
        .is_ok();
    e.check("alias_tunnel_attached", attached, "");
    let config = insecure_config()?;
    let body = exchange(&config, &rig.lab.address, DOMAIN, "GET", "/", "", "")
        .map(|(_, _, body)| String::from_utf8_lossy(&body).into_owned());
    e.check(
        "custom_sni_routes_through_relay",
        body.as_deref() == Some("alias-origin"),
        "",
    );
    ct_proof(rig, e)
}

fn ct_proof(rig: &Rig, e: &mut Evidence) -> Result<()> {
    let hooks = rig.lab.hooks();
    let a = device(&rig.a);
    lock(&hooks.ct_assigned).clone_from(&a.host);
    rig.lab.monitor()?;
    let seen = lock(&hooks.ct_seen).clone();
    e.check(
        "ct_checks_assigned_and_custom_names",
        seen.contains(&a.host) && seen.iter().any(|n| n == DOMAIN),
        "",
    );
    let header = proof_json(&rig.lab.api, &rig.first, &a, "ct")
        .map(|proof| b64(proof.to_string().as_bytes()));
    let warning = header
        .and_then(|header| {
            http_with(
                &rig.lab.api,
                "GET",
                "/ct",
                None,
                None,
                &[("X-Bridge-Proof", &header)],
            )
            .ok()
        })
        .map(|reply| {
            reply.json()["warning"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        });
    let Some(warning) = warning else {
        e.check("assigned_name_unexpected_certificate_warns", false, "");
        return Ok(());
    };
    let unexpected = format!("{}: Unexpected certificate", a.host);
    e.check(
        "assigned_name_unexpected_certificate_warns",
        warning.contains(&unexpected),
        "",
    );
    e.check(
        "ct_aggregates_name_warnings",
        warning.contains(&format!(
            "{DOMAIN}: Certificate Transparency check unavailable"
        )) && warning.contains(&unexpected),
        "",
    );
    Ok(())
}

pub fn run(world: &mut World, e: &mut Evidence) -> Result<()> {
    let id = e.scenario.clone();
    let name = format!("relay-{}", id.to_ascii_lowercase());
    let Some(rig) = prologue(world, e, &name)? else {
        return Ok(());
    };
    let device = device(&rig.a);
    let custom = id == "RELAY-OWN-DOMAIN";
    if custom {
        own_domain(&rig, e)?;
    } else {
        dns_proofs(&rig, e);
    }
    let stage = rig.directory.join("pebble");
    fs::create_dir_all(&stage)?;
    if let Err(error) = pebble_stage(&stage, &rig.lab, &rig.first, &device, custom, e) {
        let label = if custom {
            "own_domain_certificate"
        } else {
            "pebble_issue_renew"
        };
        e.check(&format!("{label}: {error}"), false, "");
    }
    let _ = &rig.fixture;
    Ok(())
}
