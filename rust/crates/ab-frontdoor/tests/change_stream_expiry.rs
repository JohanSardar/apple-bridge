#![allow(clippy::unwrap_used)]
use ab_frontdoor::{Audit, AuditEntry, Config, Frontdoor};
use ab_operations::{
    Code, Command, CommandTarget, Cursor, Error, ForwardReply, GatewayRoute, GatewayTransport,
    Identity, Principal, ReadQuery, ReadStore, RemoteGateway, Service, Task, Visibility,
};
use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
};
use serde_json::Value;
use serde_json::value::RawValue;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

#[derive(Debug)]
struct Expiring(Option<jiff::Timestamp>);
impl Identity for Expiring {
    fn authenticate(&self, _: &str) -> Result<Principal, Error> {
        Ok(Principal {
            id: "agent:synthetic".into(),
            kind: "agent".into(),
            grants: vec!["messages:read".into()],
            level: "read".into(),
            resource: String::new(),
            remote: false,
            expires_at: self.0,
        })
    }
    fn current(&self, principal: &Principal) -> Result<Principal, Error> {
        Ok(principal.clone())
    }
    fn sign(&self, _: &Principal, _: &str, _: &str) -> Result<String, Error> {
        Err(Code::Unsupported.into())
    }
}
#[derive(Debug)]
struct Quiet;
impl ReadStore for Quiet {
    fn head(&self) -> Result<Cursor, Error> {
        Ok(Cursor {
            epoch: "synthetic".into(),
            subscription_revision: 1,
            sequence: 0,
        })
    }
    fn read(&self, _: &Visibility, _: &ReadQuery) -> Result<Box<RawValue>, Error> {
        Err(Code::Unavailable.into())
    }
    fn read_model(&self, _: &Visibility, _: &str, _: &Value) -> Result<Box<RawValue>, Error> {
        Err(Code::Unavailable.into())
    }
    fn changes(&self, _: &Visibility, _: &Cursor) -> Result<Option<Box<RawValue>>, Error> {
        Ok(None)
    }
}
#[derive(Debug)]
struct Plain;
impl ab_operations::Codec for Plain {
    fn json(&self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(raw.to_vec())
    }
    fn command_hash(&self, _: &Command) -> Result<String, Error> {
        Ok("hash".into())
    }
    fn fingerprint(&self, _: &[u8]) -> String {
        "hash".into()
    }
}
#[derive(Debug)]
struct Offline;
impl GatewayTransport for Offline {
    fn connected(&self) -> bool {
        false
    }
    fn exchange<'a>(
        &'a self,
        _: &'a GatewayRoute,
        _: String,
        _: Vec<u8>,
    ) -> Task<'a, ForwardReply> {
        Box::pin(async { Err(Code::Unavailable.into()) })
    }
}
#[derive(Debug)]
struct NoAudit;
impl Audit for NoAudit {
    fn record(&self, _: AuditEntry) -> Result<(), Error> {
        Ok(())
    }
}

async fn stream_body(expires_at: Option<jiff::Timestamp>, within: Duration) -> Option<Vec<u8>> {
    let identity: Arc<dyn Identity> = Arc::new(Expiring(expires_at));
    let store: Arc<dyn ReadStore> = Arc::new(Quiet);
    let codec: Arc<dyn ab_operations::Codec> = Arc::new(Plain);
    let service = Arc::new(Service {
        store: store.clone(),
        identity: identity.clone(),
        target: CommandTarget::Remote(Arc::new(RemoteGateway {
            route: GatewayRoute::Feed,
            transport: Arc::new(Offline),
            identity,
            store,
            codec: codec.clone(),
        })),
        codec,
    });
    let router = Frontdoor::new(
        service,
        Config {
            hosts: vec!["bridge.test".into()],
            origins: vec![],
            public: false,
            requests_per_second: 100,
        },
        Arc::new(NoAudit),
    )
    .router();
    let response = router
        .oneshot(
            Request::get("/v1/changes")
                .header(header::HOST, "bridge.test")
                .header(header::AUTHORIZATION, "Bearer synthetic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    tokio::time::timeout(within, to_bytes(response.into_body(), 1 << 20))
        .await
        .ok()
        .map(|bytes| bytes.unwrap().to_vec())
}

#[tokio::test]
async fn a_change_stream_closes_when_its_access_token_expires() {
    let soon = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(1))
        .unwrap();
    let body = stream_body(Some(soon), Duration::from_secs(8)).await;
    assert!(body.is_some(), "the stream kept running past token expiry");
}

#[tokio::test]
async fn a_change_stream_without_an_expiry_stays_open() {
    assert!(stream_body(None, Duration::from_secs(2)).await.is_none());
}
