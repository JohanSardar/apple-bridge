use crate::{Result, ca::Ca};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    net::TcpListener,
    sync::{Arc, mpsc},
    time::Duration,
};

pub fn replay_gateway(ca: &Ca, challenge: String) -> Result<(String, mpsc::Receiver<bool>)> {
    let (chain_pem, key_pem) = ca.leaf()?;
    let chain = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| crate::Error::Certificate(e.to_string()))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| crate::Error::Certificate(e.to_string()))?;
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_no_client_auth()
            .with_single_cert(chain, key)?,
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = (|| -> Option<bool> {
            let (tcp, _) = listener.accept().ok()?;
            tcp.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
            tcp.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
            let stream = StreamOwned::new(ServerConnection::new(config).ok()?, tcp);
            let mut socket = tungstenite::accept(stream).ok()?;
            socket.read().ok()?;
            socket.send(tungstenite::Message::text(challenge)).ok()?;
            let message = socket.read().ok()?;
            let tungstenite::Message::Text(text) = message else {
                return Some(false);
            };
            let value: serde_json::Value = serde_json::from_str(text.as_str()).ok()?;
            Some(value.get("t").and_then(|t| t.as_str()) == Some("AuthProof"))
        })();
        let _ = sender.send(outcome.unwrap_or(false));
    });
    Ok((address, receiver))
}
