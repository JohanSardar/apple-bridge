use ab_replication::{gateway::StateAdapter, server::Server};
use ab_trust::{Authority, Grant};
use std::sync::{Arc, Mutex as BlockingMutex};
use tokio::sync::{Mutex, watch};

fn capture(version: u64) -> Result<ab_trust::wire::messages::Capture, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "source":"reminders", "generation":"fixture", "checkpoint":version.to_string(), "predecessor":if version == 1 { "" } else { "1" },
        "observations":[{"kind":"upsert","collection":"reminders.list","native_key":"fixture-list",
        "observed_at":"2026-01-02T00:00:00Z", "data":{"title":format!("fixture-{version}"),
        "blob":{"digest":ab_trust::fingerprint(b"fixture"),"size":7}}}]
    }))
}
#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let mut state = ab_state::State::<ab_state::Gateway>::open(&home)?;
    if state.head()?.sequence == 0 {
        state.commit_capture(&capture(1)?)?;
    }
    let state = Arc::new(BlockingMutex::new(state));
    if std::env::args().nth(2).as_deref() == Some("seed") {
        let mut log = ab_trust::activity::ActivityLog::open(&home)?;
        let mut activity = ab_trust::activity::AgentActivity {
            sequence: 0,
            id: "agent:fixture:read".into(),
            principal: "agent:fixture".into(),
            operation: "reminders.lists".into(),
            kind: "read".into(),
            state: "ok".into(),
            at: ab_trust::wire::gojson::GoTime::parse(&jiff::Timestamp::now().to_string())?,
            count: 0,
            argument_bytes: 2,
            argument_hash: ab_trust::fingerprint(b"{}"),
        };
        log.record(&activity)?;
        log.record(&activity)?;
        activity.id = "agent:fixture:command".into();
        activity.kind = "command".into();
        activity.state = "resolved".into();
        activity.sequence = 5;
        log.record(&activity)?;
        activity.state = "dispatched".into();
        activity.sequence = 4;
        log.record(&activity)?;
        drop(log);
        let log = ab_trust::activity::ActivityLog::open(&home)?;
        let rows = log.list("agent:fixture", "")?;
        if rows.len() != 2
            || !rows.iter().any(|r| r.kind == "read" && r.count == 2)
            || !rows
                .iter()
                .any(|r| r.kind == "command" && r.state == "resolved")
        {
            return Err("activity persistence".into());
        }
    }
    if std::env::args().nth(2).as_deref() == Some("seed") {
        return Ok(());
    }
    let authority = Arc::new(Authority::open(&home)?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let code = authority.pair(
        &listener.local_addr()?.to_string(),
        &std::env::var("GC_GO_DIAL").unwrap_or_default(),
        vec![
            Grant::parse("reminders:read")?,
            Grant::parse("messages:write")?,
        ],
        600,
    )?;
    println!("{code}");
    println!(
        "{}",
        authority.pair(
            &listener.local_addr()?.to_string(),
            &std::env::var("GC_RUST_DIAL").unwrap_or_default(),
            vec![
                Grant::parse("reminders:read")?,
                Grant::parse("messages:write")?
            ],
            600
        )?
    );
    let mut blobs = ab_blobs::Store::open(&home, 0)?;
    blobs
        .transfer(
            &FixtureBlob,
            "fixture-handle",
            &ab_blobs::Digest::try_from(ab_trust::fingerprint(b"fixture"))?,
            7,
        )
        .await?;
    let blobs = Arc::new(Mutex::new(blobs));
    tokio::task::block_in_place(|| {
        state
            .lock()
            .map_err(|_| "state mutex")?
            .set_blob_protection(Arc::new(ab_replication::gateway::ProtectionAdapter(
                blobs.clone(),
            )))
            .map_err(|_| "protection")
    })?;
    let uploads = Arc::new(ab_replication::upstream::Uploads::new(
        authority.clone(),
        blobs.clone(),
    ));
    let server = Arc::new(Server {
        uploads: Some(uploads),
        blobs: Some(blobs),
        state: Arc::new(Mutex::new(StateAdapter(state.clone()))),
        authority,
    });
    let (_stop, rx) = watch::channel(false);
    let advance = std::path::Path::new(&home).join("advance");
    tokio::spawn(async move {
        while !advance.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::task::spawn_blocking(move || {
            state
                .lock()
                .map_err(|_| "poisoned")?
                .commit_capture(&capture(2)?)?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await
    });
    tokio::select! { result = server.clone().listen(listener, rx.clone()) => result?, result = server.dial(rx) => result? };
    Ok(())
}

struct FixtureBlob;
impl ab_blobs::BlobRead for FixtureBlob {
    fn read(
        &self,
        handle: &str,
        offset: u64,
        length: usize,
    ) -> impl std::future::Future<Output = Result<(Vec<u8>, bool), ab_blobs::Error>> + Send {
        std::future::ready((|| {
            let offset = usize::try_from(offset).map_err(|_| ab_blobs::Error::Invalid)?;
            if handle != "fixture-handle" || offset > 7 || length == 0 {
                return Err(ab_blobs::Error::Invalid);
            }
            Ok((b"fixture"[offset..].to_vec(), true))
        })())
    }
}
