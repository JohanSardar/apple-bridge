use crate::{
    Error,
    server::{GatewayState, Server},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

impl<S: GatewayState + 'static> Server<S> {
    pub async fn dial(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
        let mut active = std::collections::HashSet::new();
        let mut tasks = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Ok(id) = result { active.remove(&id); }
                },
                _ = tick.tick() => {
                    self.authority.dial_pairings_once().await?;
                    for principal in self.authority.dial_replicas()? {
                        if active.contains(&principal.id) || tasks.len() >= 64 { continue; }
                        active.insert(principal.id.clone());
                        let server = self.clone();
                        tasks.spawn(async move {
                            let _ = server.dial_replica(&principal).await;
                            principal.id
                        });
                    }
                }
            }
        }
    }
    async fn dial_replica(&self, principal: &ab_trust::Principal) -> Result<(), Error> {
        let mut backoff = Duration::from_millis(250);
        while self.authority.session_valid(principal)? {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let stream = tokio::net::TcpStream::connect(&principal.dial).await?;
                let host = principal
                    .dial
                    .rsplit_once(':')
                    .ok_or(Error::Invalid)?
                    .0
                    .trim_matches(['[', ']'])
                    .to_owned();
                let name = tokio_rustls::rustls::pki_types::ServerName::try_from(host)
                    .map_err(|_| Error::Invalid)?;
                let tls = tokio_rustls::TlsConnector::from(Arc::new(
                    self.authority.client_tls(&principal.fingerprint)?,
                ));
                let stream = tls.connect(name, stream).await?;
                let cert = stream
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|c| c.first())
                    .ok_or(Error::Invalid)?
                    .to_vec();
                let current = self.authority.authenticate_certificate(&cert)?;
                if current.id != principal.id {
                    return Err(Error::Invalid);
                }
                let (socket, _) = tokio_tungstenite::client_async(
                    format!("wss://{}/v1/feed", principal.dial),
                    stream,
                )
                .await?;
                Ok::<_, Error>((socket, cert))
            })
            .await;
            if let Ok(Ok((socket, cert))) = result {
                self.authority.confirm_dial(&principal.id)?;
                let _ = self.serve(socket, &cert).await;
                backoff = Duration::from_millis(250);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
        Ok(())
    }
}
