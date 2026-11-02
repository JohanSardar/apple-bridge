use ab_operations::{Code, Error, ForwardReply, GatewayRoute, GatewayTransport, Task};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[derive(Debug)]
pub struct DirectTransport {
    pub authority: Arc<ab_trust::Authority>,
    pub connected: Arc<AtomicBool>,
}
impl GatewayTransport for DirectTransport {
    fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    fn exchange<'a>(
        &'a self,
        route: &'a GatewayRoute,
        delegation: String,
        body: Vec<u8>,
    ) -> Task<'a, ForwardReply> {
        Box::pin(async move {
            let GatewayRoute::Direct {
                address,
                fingerprint,
            } = route
            else {
                return Err(Code::Unavailable.into());
            };
            let tls = self
                .authority
                .client_tls(fingerprint)
                .map_err(|_| Error::from(Code::Unavailable))?;
            let client = reqwest::Client::builder()
                .use_preconfigured_tls(tls)
                .connect_timeout(Duration::from_millis(1500))
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::from(Code::Unavailable))?;
            let mut response = client
                .post(format!("https://{address}/v1/commands"))
                .header("X-Apple-Bridge-Delegation", delegation)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| Error::from(Code::Unavailable))?;
            let lookup_only = response
                .headers()
                .get("X-Apple-Bridge-Command-Lookup-Only")
                .is_some_and(|h| h == "true");
            let success = response.status().is_success();
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| Error::from(Code::Unavailable))?
            {
                if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                    return Err(Code::Corrupt.into());
                }
                bytes.extend_from_slice(&chunk);
            }
            if !success {
                #[derive(Deserialize)]
                struct Envelope {
                    error: Error,
                }
                return Err(serde_json::from_slice::<Envelope>(&bytes).map_or_else(
                    |_| Error::from(Code::Unavailable),
                    |envelope| envelope.error,
                ));
            }
            let result: Box<RawValue> =
                serde_json::from_slice(&bytes).map_err(|_| Error::from(Code::Corrupt))?;
            Ok(ForwardReply {
                result,
                lookup_only,
            })
        })
    }
}
