use ab_operations::{Code, Error, ForwardReply, GatewayRoute, GatewayTransport, Task};
#[derive(Debug)]
pub struct ReplicaGatewayTransport(pub ab_replication::GatewayTransport);
impl GatewayTransport for ReplicaGatewayTransport {
    fn connected(&self) -> bool {
        self.0.connected()
    }
    fn exchange<'a>(
        &'a self,
        route: &'a GatewayRoute,
        delegation: String,
        body: Vec<u8>,
    ) -> Task<'a, ForwardReply> {
        Box::pin(async move {
            let route = match route {
                GatewayRoute::Feed => ab_replication::GatewayRoute::Feed,
                GatewayRoute::Direct {
                    address,
                    fingerprint,
                } => ab_replication::GatewayRoute::Direct {
                    address: address.clone(),
                    fingerprint: fingerprint.clone(),
                },
            };
            let response = self
                .0
                .exchange(&route, delegation, body)
                .await
                .map_err(|error| match error {
                    ab_replication::Error::Trust(ab_trust::Error::Http(error))
                        if error.is_timeout() =>
                    {
                        Error::from(Code::Deadline)
                    }
                    _ => Error::from(Code::Unavailable),
                })?;
            if let Some(error) = response.error {
                return Err(serde_json::to_vec(&error)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_else(|| Code::Corrupt.into()));
            }
            Ok(ForwardReply {
                result: response.result.ok_or(Code::Corrupt)?,
                lookup_only: response.lookup_only,
            })
        })
    }
}
