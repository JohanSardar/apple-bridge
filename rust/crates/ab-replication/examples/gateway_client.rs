use ab_replication::Client;
use ab_trust::Authority;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let code = std::env::args().nth(2).ok_or("code")?;
    let authority = Arc::new(Authority::open(&home)?);
    if code != "-" {
        authority.join(&code).await?;
    }
    let state = Arc::new(Mutex::new(ab_state::State::<ab_state::Replica>::open(
        &home,
    )?));
    let client = Client::new(
        state.clone(),
        authority,
        Some(Arc::new(Mutex::new(ab_blobs::Store::open(&home, 0)?))),
    );
    let (_stop, rx) = watch::channel(false);
    let mut status = client.status();
    let monitor = async {
        loop {
            tokio::select! { r = status.changed() => r?, () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {} }
            let state = state.lock().await;
            let records = state.read(
                &ab_trust::Principal {
                    grants: vec![ab_trust::Grant::parse("reminders:read")?],
                    ..ab_trust::Principal::default()
                }
                .visibility()
                .to_map(),
                &ab_state::ReadQuery::default(),
            )?;
            ab_trust::atomic_write(
                &std::path::Path::new(&home).join("state.json"),
                &serde_json::to_vec(
                    &serde_json::json!({"cursor":records.cursor,"digest":ab_state::digest(&records.records)}),
                )?,
            )?;
            ab_trust::atomic_write(
                &std::path::Path::new(&home).join("status.json"),
                &serde_json::to_vec(&*status.borrow())?,
            )?;
        }
        #[allow(unreachable_code)]
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    tokio::select! { result = client.follow(rx) => result?, result = monitor => result? }
    Ok(())
}
