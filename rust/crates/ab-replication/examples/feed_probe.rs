use ab_replication::Client;
use ab_trust::{Authority, Principal};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, watch};
#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let home = PathBuf::from(args.get(1).ok_or("home")?);
    let authority = Arc::new(Authority::open(&home)?);
    if let Some(code) = args.get(2).filter(|c| !c.is_empty()) {
        if let Some(address) = args.get(3) {
            authority.join_listen(code, address).await?;
        } else {
            authority.join(code).await?;
        }
    }
    if !home.join("probe-token").exists() {
        let token =
            authority.create_token(vec![ab_trust::Grant::parse("reminders:read")?], false)?;
        let principal = authority.authenticate_token(&token)?;
        ab_trust::atomic_write(&home.join("probe-token"), token.as_bytes())?;
        ab_trust::atomic_write(
            &home.join("probe-delegation"),
            authority
                .sign_delegation(
                    &principal,
                    "interop",
                    &ab_trust::fingerprint(b"interop"),
                    "",
                )?
                .as_bytes(),
        )?;
    }
    let state = Arc::new(Mutex::new(ab_state::State::<ab_state::Replica>::open(
        &home,
    )?));
    let client = Client::new(
        state.clone(),
        authority.clone(),
        Some(Arc::new(Mutex::new(ab_blobs::Store::open(&home, 0)?))),
    );
    let (stop, rx) = watch::channel(false);
    let _stop = stop;
    let mut status = client.status();
    let monitor = async {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {r=status.changed()=>r?,_=tick.tick()=>{}}
            let visible = Principal {
                grants: vec![ab_trust::Grant::parse("admin")?],
                ..Principal::default()
            }
            .visibility()
            .to_map();
            let result = state.lock().await.read(
                &visible,
                &ab_state::ReadQuery {
                    include_deleted: true,
                    limit: 10000,
                    ..ab_state::ReadQuery::default()
                },
            )?;
            let records: std::collections::BTreeMap<_, _> = result
                .records
                .into_iter()
                .map(|r| (r.id.as_str().to_owned(), r))
                .collect();
            let output = serde_json::json!({"cursor":result.cursor,"records":records,"partial_batches":status.borrow().partial_batches});
            ab_trust::atomic_write(
                &home.join("probe-state.json"),
                &serde_json::to_vec(&output)?,
            )?;
            ab_trust::atomic_write(
                &home.join("probe-status.json"),
                &serde_json::to_vec(&*status.borrow())?,
            )?;
        }
        #[allow(unreachable_code)]
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let command_checks = async {
        let transport = client.gateway_transport();
        while !transport.connected() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let token = std::fs::read_to_string(home.join("probe-token"))?;
        let principal = authority.authenticate_token(&token)?;
        let enrollment = authority.enrollment()?.ok_or("enrollment")?;
        let routes = [
            ab_replication::GatewayRoute::Feed,
            ab_replication::GatewayRoute::Direct {
                address: enrollment.address,
                fingerprint: enrollment.fingerprint,
            },
        ];
        for route in &routes {
            let body=br#"{"method":"list","command":{"args":null,"expires_at":"0001-01-01T00:00:00Z"},"query":{}}"#.to_vec();
            let delegation =
                authority.sign_delegation(&principal, "", &ab_trust::fingerprint(&body), "")?;
            let reply = transport.exchange(route, delegation, body).await?;
            if reply.error.is_some() || reply.result.as_ref().is_none_or(|r| r.get() != "[]") {
                return Err("forwarded list failed".into());
            }
            let body=br#"{"method":"status","command":{"args":null,"expires_at":"0001-01-01T00:00:00Z"},"query":{"command_id":"missing"}}"#.to_vec();
            let delegation = authority.sign_delegation(
                &principal,
                "missing",
                &ab_trust::fingerprint(&body),
                "",
            )?;
            let reply = transport.exchange(route, delegation, body).await?;
            if reply
                .error
                .as_ref()
                .is_none_or(|e| e.code != ab_trust::domain::ErrorCode::NotFound)
            {
                return Err("forwarded error failed".into());
            }
        }
        ab_trust::atomic_write(
            &home.join("probe-commands.json"),
            br#"{"feed":true,"direct_mtls":true,"error_envelope":true}"#,
        )?;
        std::future::pending::<()>().await;
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let run = async {
        if let Some(address) = args.get(3) {
            client.listen(address, rx).await
        } else {
            client.follow(rx).await
        }
    };
    tokio::select! {r=run=>r?,r=monitor=>r?,r=command_checks=>r?}
    Ok(())
}
