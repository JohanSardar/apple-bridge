use crate::Error;
use ab_operations::GatewayStore;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

pub async fn run(
    state: GatewayStore,
    blobs: Arc<Mutex<ab_blobs::Store>>,
    grace_ms: u64,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut gc = tokio::time::interval(std::time::Duration::from_secs(1));
    let grace = if grace_ms == 0 {
        24 * 60 * 60
    } else {
        i64::try_from(grace_ms.div_ceil(1000)).map_err(|_| Error::Invalid)?
    };
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tick.tick() => {
                let entries = blobs.lock().await.entries().clone();
                state.with(|state| {
                    let cursor = state.head()?;
                    state.set_replica_status(serde_json::json!({"role":"gateway","state":"ready","cursor":cursor,"gateway_head":cursor.sequence,"schema":2,"lag":0,"staleness_ms":0,"blobs":{"entries":entries}}))
                })?;
            },
            _ = gc.tick() => {
                let mut blobs = blobs.clone().lock_owned().await;
                tokio::task::spawn_blocking(move || blobs.gc(jiff::Timestamp::now(), grace)).await.map_err(|_| Error::Invalid)?.map_err(|_| Error::Invalid)?;
            }
        }
    }
}
