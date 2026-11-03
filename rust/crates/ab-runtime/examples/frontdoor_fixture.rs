#![forbid(unsafe_code)]
use std::sync::{Arc, Mutex};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let home = std::env::temp_dir().join(format!("ab-product-frontdoor-{}", std::process::id()));
    std::fs::create_dir_all(&home)?;
    let authority = Arc::new(ab_trust::Authority::open(&home)?);
    let token = authority.create_token(vec![ab_trust::Grant::parse("admin")?], true)?;
    let state = Arc::new(Mutex::new(ab_state::State::<ab_state::Replica>::open(
        &home,
    )?));
    let blobs = Arc::new(tokio::sync::Mutex::new(ab_blobs::Store::open(&home, 0)?));
    let client = ab_replication::Client::new(
        Arc::new(tokio::sync::Mutex::new(
            ab_state::State::<ab_state::Replica>::open(home.join("feed"))?,
        )),
        authority.clone(),
        None,
    );
    let reads = Arc::new(ab_operations::StateReads(state, blobs));
    let identity = Arc::new(ab_operations::TrustIdentity(authority));
    let codec = Arc::new(ab_runtime::GoJson);
    let remote = Arc::new(ab_operations::RemoteGateway {
        route: ab_operations::GatewayRoute::Feed,
        transport: Arc::new(ab_runtime::transport::ReplicaGatewayTransport(
            client.gateway_transport(),
        )),
        identity: identity.clone(),
        store: reads.clone(),
        codec: codec.clone(),
    });
    let service = Arc::new(ab_operations::Service {
        store: reads,
        identity,
        target: ab_operations::CommandTarget::Remote(remote),
        codec,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let frontdoor = ab_frontdoor::Frontdoor::new(
        service,
        ab_frontdoor::Config {
            hosts: vec!["localhost".into()],
            origins: vec![],
            public: mode == "sse",
            requests_per_second: 200,
        },
        Arc::new(ab_runtime::AuditLog::open(&home)?),
    );
    println!(
        "{}",
        serde_json::json!({"address":address.to_string(),"token":token})
    );
    axum::serve(
        listener,
        frontdoor
            .clone()
            .router()
            .merge(ab_frontdoor::golden_router(frontdoor)),
    )
    .await?;
    Ok(())
}
