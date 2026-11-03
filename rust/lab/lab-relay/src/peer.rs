use crate::{Error, Identity, Result, fingerprint};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

#[derive(Debug)]
struct Pin {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("gateway pin mismatch".into()))
        }
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("tls 1.2 is not allowed".into()))
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
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
#[derive(Debug)]
pub struct PeerReply {
    pub status: u16,
    pub body: Vec<u8>,
}
pub fn peer_post(
    address: &str,
    identity: &Identity,
    pin: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<PeerReply> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pin {
            fingerprint: pin.to_owned(),
            provider,
        }))
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.pkcs8()?)),
        )?;
    let target = address.to_socket_addrs()?.next().ok_or(Error::Invalid)?;
    let tcp = TcpStream::connect_timeout(&target, Duration::from_secs(5))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))?;
    let connection = ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost").map_err(|_| Error::Invalid)?,
    )?;
    let mut stream = StreamOwned::new(connection, tcp);
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut raw = Vec::new();
    match stream.read_to_end(&mut raw) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
        Err(error) => return Err(error.into()),
    }
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(Error::Invalid)?;
    let status = std::str::from_utf8(&raw[..split])
        .ok()
        .and_then(|head| head.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or(Error::Invalid)?;
    Ok(PeerReply {
        status,
        body: raw[split + 4..].to_vec(),
    })
}
