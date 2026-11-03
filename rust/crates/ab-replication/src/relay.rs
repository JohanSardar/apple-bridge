use crate::Error;
use ab_trust::{Authority, Principal};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::Verifier;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

pub const LABEL: &[u8] = b"EXPORTER-Channel-Binding";
const VERSION: &str = "relay/2";
const CAPS: &str = "tls-exporter,feed/1";
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub t: String,
    pub version: String,
    pub capabilities: String,
    #[serde(default)]
    pub nonce: String,
    #[serde(default)]
    pub certificate: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub pairing_id: String,
    #[serde(default)]
    pub proof: String,
    #[serde(default)]
    pub principal: Option<Box<serde_json::value::RawValue>>,
}
fn frame(t: &str) -> Frame {
    Frame {
        t: t.into(),
        version: VERSION.into(),
        capabilities: CAPS.into(),
        ..Frame::default()
    }
}
fn nonce() -> String {
    let mut n = [0; 32];
    rand::thread_rng().fill_bytes(&mut n);
    STANDARD.encode(n)
}
fn input(
    role: &str,
    hello: &Frame,
    challenge: &Frame,
    binding: &[u8; 32],
    principal: Option<&serde_json::value::RawValue>,
) -> Result<Vec<u8>, Error> {
    let mut out = format!("apple-bridge-relay/2\0{role}\0{VERSION}\0{CAPS}\0").into_bytes();
    out.extend_from_slice(binding);
    for n in [&hello.nonce, &challenge.nonce] {
        let n = STANDARD.decode(n).map_err(|_| Error::Invalid)?;
        if n.len() != 32 {
            return Err(Error::Invalid);
        }
        out.extend_from_slice(&n);
    }
    for bytes in [
        STANDARD
            .decode(&hello.certificate)
            .map_err(|_| Error::Invalid)?,
        STANDARD
            .decode(&challenge.certificate)
            .map_err(|_| Error::Invalid)?,
        hello.pairing_id.as_bytes().to_vec(),
        principal.map_or_else(Vec::new, |principal| principal.get().as_bytes().to_vec()),
    ] {
        out.extend_from_slice(&Sha256::digest(bytes));
    }
    Ok(out)
}
fn sign(
    authority: &Authority,
    role: &str,
    hello: &Frame,
    challenge: &Frame,
    binding: &[u8; 32],
    principal: Option<&serde_json::value::RawValue>,
) -> Result<String, Error> {
    Ok(STANDARD.encode(
        authority.sign_relay_transcript(&input(role, hello, challenge, binding, principal)?)?,
    ))
}
fn verify(
    cert: &str,
    sig: &str,
    role: &str,
    hello: &Frame,
    challenge: &Frame,
    binding: &[u8; 32],
    principal: Option<&serde_json::value::RawValue>,
) -> Result<(), Error> {
    let der = STANDARD.decode(cert).map_err(|_| Error::Invalid)?;
    let (_, cert) = x509_parser::parse_x509_certificate(&der).map_err(|_| Error::Invalid)?;
    if !cert.validity().is_valid() {
        return Err(Error::Invalid);
    }
    let key = ed25519_dalek::VerifyingKey::from_bytes(
        cert.public_key()
            .subject_public_key
            .data
            .as_ref()
            .try_into()
            .map_err(|_| Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)?;
    let sig = STANDARD.decode(sig).map_err(|_| Error::Invalid)?;
    key.verify(
        &input(role, hello, challenge, binding, principal)?,
        &ed25519_dalek::Signature::from_slice(&sig).map_err(|_| Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)
}
async fn read<T: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<T>,
    t: &str,
) -> Result<Frame, Error> {
    let msg = ws.next().await.ok_or(Error::Disconnected)??;
    let Message::Text(text) = msg else {
        return Err(Error::Invalid);
    };
    if text.len() > 8192 {
        return Err(Error::Invalid);
    }
    let f: Frame = serde_json::from_str(&text)?;
    if f.t != t || f.version != VERSION || f.capabilities != CAPS {
        return Err(Error::Invalid);
    }
    Ok(f)
}
async fn write<T: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<T>,
    f: &Frame,
) -> Result<(), Error> {
    ws.send(Message::Text(serde_json::to_string(f)?.into()))
        .await?;
    Ok(())
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyFrame {
    t: String,
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    certificate: String,
    #[serde(default)]
    signature: String,
    #[serde(default)]
    pairing_id: String,
    #[serde(default)]
    proof: String,
    #[serde(default)]
    principal: Option<Box<serde_json::value::RawValue>>,
}
fn legacy_input(
    role: &str,
    hello: &LegacyFrame,
    challenge: &LegacyFrame,
    principal: Option<&serde_json::value::RawValue>,
) -> Result<Vec<u8>, Error> {
    let mut out = format!("apple-bridge-relay/1\0{role}\0").into_bytes();
    for n in [&hello.nonce, &challenge.nonce] {
        let n = STANDARD.decode(n).map_err(|_| Error::Invalid)?;
        if n.len() != 32 {
            return Err(Error::Invalid);
        }
        out.extend_from_slice(&n);
    }
    for bytes in [
        STANDARD
            .decode(&hello.certificate)
            .map_err(|_| Error::Invalid)?,
        STANDARD
            .decode(&challenge.certificate)
            .map_err(|_| Error::Invalid)?,
        hello.pairing_id.as_bytes().to_vec(),
        principal.map_or_else(Vec::new, |principal| principal.get().as_bytes().to_vec()),
    ] {
        out.extend_from_slice(&Sha256::digest(bytes));
    }
    Ok(out)
}
fn legacy_decode(text: &str, t: &str) -> Result<LegacyFrame, Error> {
    let f: LegacyFrame = serde_json::from_str(text)?;
    if f.t != t {
        return Err(Error::Invalid);
    }
    Ok(f)
}
async fn next_text<T: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<T>,
) -> Result<String, Error> {
    let msg = ws.next().await.ok_or(Error::Disconnected)??;
    let Message::Text(text) = msg else {
        return Err(Error::Invalid);
    };
    if text.len() > 8192 {
        return Err(Error::Invalid);
    }
    Ok(text.as_str().to_owned())
}
async fn legacy_server<T: AsyncRead + AsyncWrite + Unpin>(
    authority: &Authority,
    ws: &mut WebSocketStream<T>,
    hello: LegacyFrame,
) -> Result<Vec<u8>, Error> {
    let der = STANDARD
        .decode(&hello.certificate)
        .map_err(|_| Error::Invalid)?;
    let mut challenge = LegacyFrame {
        t: "AuthChallenge".into(),
        nonce: nonce(),
        certificate: STANDARD.encode(&authority.certificate),
        ..LegacyFrame::default()
    };
    challenge.signature = STANDARD.encode(
        authority.sign_relay_transcript(&legacy_input("gateway", &hello, &challenge, None)?)?,
    );
    ws.send(Message::Text(serde_json::to_string(&challenge)?.into()))
        .await?;
    let proof = legacy_decode(&next_text(ws).await?, "AuthProof")?;
    let (_, cert) = x509_parser::parse_x509_certificate(&der).map_err(|_| Error::Invalid)?;
    if !cert.validity().is_valid() {
        return Err(Error::Invalid);
    }
    let key = ed25519_dalek::VerifyingKey::from_bytes(
        cert.public_key()
            .subject_public_key
            .data
            .as_ref()
            .try_into()
            .map_err(|_| Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)?;
    let sig = STANDARD
        .decode(&proof.signature)
        .map_err(|_| Error::Invalid)?;
    key.verify(
        &legacy_input("replica", &hello, &challenge, None)?,
        &ed25519_dalek::Signature::from_slice(&sig).map_err(|_| Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)?;
    let principal = if hello.pairing_id.is_empty() {
        let principal = authority.authenticate_certificate(&der)?;
        if principal.relay2() {
            return Err(Error::Invalid);
        }
        principal
    } else {
        authority.accept_pair(
            &hello.pairing_id,
            &STANDARD.decode(proof.proof).map_err(|_| Error::Invalid)?,
            &der,
            false,
        )?
    };
    let raw = serde_json::value::to_raw_value(&principal)?;
    let mut accepted = LegacyFrame {
        t: "Authenticated".into(),
        ..LegacyFrame::default()
    };
    accepted.signature = STANDARD.encode(authority.sign_relay_transcript(&legacy_input(
        "accepted",
        &hello,
        &challenge,
        Some(&raw),
    )?)?);
    accepted.principal = Some(raw);
    ws.send(Message::Text(serde_json::to_string(&accepted)?.into()))
        .await?;
    Ok(der)
}
pub async fn server<T: AsyncRead + AsyncWrite + Unpin>(
    authority: &Authority,
    ws: &mut WebSocketStream<T>,
    binding: Option<&[u8; 32]>,
) -> Result<Vec<u8>, Error> {
    let first = next_text(ws).await?;
    let probe: serde_json::Value = serde_json::from_str(&first)?;
    if probe.get("version").is_none() {
        return legacy_server(authority, ws, legacy_decode(&first, "AuthHello")?).await;
    }
    let binding = binding.ok_or(Error::Invalid)?;
    let hello: Frame = serde_json::from_str(&first)?;
    if hello.t != "AuthHello" || hello.version != VERSION || hello.capabilities != CAPS {
        return Err(Error::Invalid);
    }
    let mut challenge = frame("AuthChallenge");
    challenge.nonce = nonce();
    challenge.certificate = STANDARD.encode(&authority.certificate);
    challenge.signature = sign(authority, "gateway", &hello, &challenge, binding, None)?;
    write(ws, &challenge).await?;
    let proof = read(ws, "AuthProof").await?;
    verify(
        &hello.certificate,
        &proof.signature,
        "replica",
        &hello,
        &challenge,
        binding,
        None,
    )?;
    let der = STANDARD
        .decode(&hello.certificate)
        .map_err(|_| Error::Invalid)?;
    let principal = if hello.pairing_id.is_empty() {
        authority.authenticate_certificate(&der)?
    } else {
        authority.accept_pair(
            &hello.pairing_id,
            &STANDARD.decode(proof.proof).map_err(|_| Error::Invalid)?,
            &der,
            false,
        )?
    };
    let principal = authority.mark_relay2(&principal.id)?;
    let mut accepted = frame("Authenticated");
    let raw = serde_json::value::to_raw_value(&principal)?;
    accepted.signature = sign(
        authority,
        "accepted",
        &hello,
        &challenge,
        binding,
        Some(&raw),
    )?;
    accepted.principal = Some(raw);
    write(ws, &accepted).await?;
    Ok(der)
}
pub async fn client<T: AsyncRead + AsyncWrite + Unpin>(
    authority: &Authority,
    ws: &mut WebSocketStream<T>,
    binding: &[u8; 32],
    pin: &str,
    pairing: Option<&ab_trust::PairCode>,
) -> Result<Principal, Error> {
    let mut hello = frame("AuthHello");
    hello.nonce = nonce();
    hello.certificate = STANDARD.encode(&authority.certificate);
    if let Some(principal) = pairing {
        hello.pairing_id.clone_from(&principal.id);
    }
    write(ws, &hello).await?;
    let challenge = read(ws, "AuthChallenge").await?;
    if ab_trust::fingerprint(
        &STANDARD
            .decode(&challenge.certificate)
            .map_err(|_| Error::Invalid)?,
    ) != pin
    {
        return Err(Error::Invalid);
    }
    verify(
        &challenge.certificate,
        &challenge.signature,
        "gateway",
        &hello,
        &challenge,
        binding,
        None,
    )?;
    let mut proof = frame("AuthProof");
    proof.signature = sign(authority, "replica", &hello, &challenge, binding, None)?;
    if let Some(principal) = pairing {
        proof.proof = principal.proof(&authority.certificate)?;
    }
    write(ws, &proof).await?;
    let accepted = read(ws, "Authenticated").await?;
    verify(
        &challenge.certificate,
        &accepted.signature,
        "accepted",
        &hello,
        &challenge,
        binding,
        accepted.principal.as_deref(),
    )?;
    let principal: Principal =
        serde_json::from_str(accepted.principal.ok_or(Error::Invalid)?.get())?;
    if principal.kind != "replica" || principal.fingerprint != authority.fingerprint {
        return Err(Error::Invalid);
    }
    Ok(principal)
}
pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
pub async fn dial(address: &str) -> Result<Socket, Error> {
    let url = url_address(address)?;
    let (ws, _) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio_tungstenite::connect_async(url),
    )
    .await
    .map_err(|_| Error::Disconnected)??;
    Ok(ws)
}
fn url_address(address: &str) -> Result<String, Error> {
    let url = reqwest::Url::parse(address).map_err(|_| Error::Invalid)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !["", "/"].contains(&url.path())
    {
        return Err(Error::Invalid);
    }
    Ok(format!(
        "wss://{}/v1/feed",
        &url[url::Position::BeforeHost..url::Position::AfterPort]
    ))
}
pub fn client_binding(ws: &Socket) -> Result<[u8; 32], Error> {
    let MaybeTlsStream::Rustls(tls) = ws.get_ref() else {
        return Err(Error::Invalid);
    };
    if tls.get_ref().1.protocol_version() != Some(tokio_rustls::rustls::ProtocolVersion::TLSv1_3) {
        return Err(Error::Invalid);
    }
    tls.get_ref()
        .1
        .export_keying_material([0; 32], LABEL, Some(&[]))
        .map_err(|_| Error::Invalid)
}

pub async fn join(authority: &Authority, code: &str) -> Result<ab_trust::Enrollment, Error> {
    let challenge = ab_trust::PairCode::decode(code)?;
    if !challenge.remote || !challenge.dial.is_empty() {
        return Err(Error::Invalid);
    }
    let mut ws = dial(&challenge.address).await?;
    let binding = client_binding(&ws)?;
    let principal = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client(
            authority,
            &mut ws,
            &binding,
            &challenge.fingerprint,
            Some(&challenge),
        ),
    )
    .await
    .map_err(|_| Error::Disconnected)??;
    let enrollment = ab_trust::Enrollment {
        remote: true,
        pairing_id: challenge.id,
        dial: String::new(),
        ceiling: principal.grants.clone(),
        replica_id: principal.id,
        address: challenge.address,
        fingerprint: challenge.fingerprint,
        grants: principal.grants,
    };
    authority.save_enrollment(enrollment.clone())?;
    Ok(enrollment)
}
