use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
};

#[derive(Debug)]
struct Pin {
    digest: String,
}
impl Pin {
    fn check(&self, cert: &CertificateDer<'_>, now: UnixTime) -> Result<(), rustls::Error> {
        if format!("{:x}", Sha256::digest(cert.as_ref())) != self.digest {
            return Err(rustls::Error::General("pin".into()));
        }
        let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
            .map_err(|_| rustls::Error::General("DER".into()))?;
        let seconds =
            i64::try_from(now.as_secs()).map_err(|_| rustls::Error::General("time".into()))?;
        if seconds < parsed.validity().not_before.timestamp()
            || seconds > parsed.validity().not_after.timestamp()
        {
            return Err(rustls::Error::General("expired".into()));
        }
        Ok(())
    }
}
impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.check(cert, now)?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
impl ClientCertVerifier for Pin {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.check(cert, now)?;
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mode = &args[2];
    let dir = &args[3];
    let pin = &args[4];
    let cert = CertificateDer::from(std::fs::read(format!("{dir}/rust.der"))?);
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(std::fs::read(format!(
        "{dir}/rust.key"
    ))?));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    if mode == "server" {
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(Pin {
                digest: pin.clone(),
            }))
            .with_single_cert(vec![cert], key)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        writeln!(std::io::stdout(), "{}", listener.local_addr()?.port())?;
        std::io::stdout().flush()?;
        let (socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        let conn = rustls::ServerConnection::new(Arc::new(config))?;
        let mut stream = rustls::StreamOwned::new(conn, socket);
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        stream.write_all(&byte)?;
        stream.flush()?;
    } else {
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pin {
                digest: pin.clone(),
            }))
            .with_client_auth_cert(vec![cert], key)?;
        let socket = TcpStream::connect(format!("127.0.0.1:{}", args[5]))?;
        socket.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        let conn = rustls::ClientConnection::new(
            Arc::new(config),
            ServerName::try_from("synthetic.invalid")?,
        )?;
        let mut stream = rustls::StreamOwned::new(conn, socket);
        stream.write_all(b"x")?;
        stream.flush()?;
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        if byte != *b"x" {
            return Err("echo".into());
        }
    }
    Ok(())
}
