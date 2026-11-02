#![forbid(unsafe_code)]
use ab_operations::{
    Code, Cursor, Error, ForwardReply, GatewayRoute, GatewayTransport, Identity, Principal,
    ReadQuery, ReadStore, Task, Visibility,
};
use serde_json::{json, value::RawValue};
use std::sync::Arc;
#[derive(Debug)]
struct Fixture;
impl Identity for Fixture {
    fn authenticate(&self, token: &str) -> Result<Principal, Error> {
        if token != "synthetic" {
            return Err(Code::Unauthorized.into());
        }
        Ok(Principal {
            id: "fixture".into(),
            kind: "token".into(),
            grants: vec!["admin".into()],
            level: String::new(),
            resource: String::new(),
            remote: true,
        })
    }
    fn current(&self, principal: &Principal) -> Result<Principal, Error> {
        Ok(principal.clone())
    }
    fn sign(&self, _: &Principal, _: &str, _: &str) -> Result<String, Error> {
        Err(Code::Unavailable.into())
    }
}
impl ReadStore for Fixture {
    fn head(&self) -> Result<Cursor, Error> {
        Ok(Cursor {
            epoch: "synthetic".into(),
            subscription_revision: 1,
            sequence: 7,
        })
    }
    fn read(&self, _: &Visibility, _: &ReadQuery) -> Result<Box<RawValue>, Error> {
        serde_json::value::to_raw_value(&json!({"records":[],"cursor":self.head()?}))
            .map_err(|_| Code::Corrupt.into())
    }
    fn read_model(
        &self,
        _: &Visibility,
        _: &str,
        _: &serde_json::Value,
    ) -> Result<Box<RawValue>, Error> {
        Err(Code::Unsupported.into())
    }
    fn changes(&self, _: &Visibility, _: &Cursor) -> Result<Option<Box<RawValue>>, Error> {
        Ok(None)
    }
}
impl GatewayTransport for Fixture {
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
struct Audit;
impl ab_frontdoor::Audit for Audit {
    fn record(&self, _: ab_frontdoor::AuditEntry) -> Result<(), Error> {
        Ok(())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Arc::new(Fixture);
    let codec = Arc::new(ab_runtime::GoJson);
    let remote = Arc::new(ab_operations::RemoteGateway {
        route: GatewayRoute::Feed,
        transport: fixture.clone(),
        identity: fixture.clone(),
        store: fixture.clone(),
        codec: codec.clone(),
    });
    let service = Arc::new(ab_operations::Service {
        store: fixture.clone(),
        identity: fixture,
        target: ab_operations::CommandTarget::Remote(remote),
        codec,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let frontdoor = ab_frontdoor::Frontdoor::new(
        service,
        ab_frontdoor::Config {
            hosts: vec![
                address.to_string(),
                "localhost".into(),
                "fixture.example.com".into(),
            ],
            origins: vec!["https://fixture.example.com:443".into()],
            public: std::env::args().any(|a| a == "sse"),
            requests_per_second: 200,
        },
        Arc::new(Audit),
    );
    println!("{address}");
    axum::serve(listener, frontdoor.router()).await?;
    Ok(())
}
