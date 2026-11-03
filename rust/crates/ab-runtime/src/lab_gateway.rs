use crate::Error;
use ab_operations::GatewayStore;
use axum::{Router, body::Body, response::IntoResponse};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::Arc,
};
use tokio::sync::{Mutex, watch};
use tower::ServiceExt;

#[derive(Debug)]
struct App {
    state: GatewayStore,
    blobs: Arc<Mutex<ab_blobs::Store>>,
    snapshots: Mutex<BTreeMap<String, String>>,
    serial: Mutex<()>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Request {
    action: String,
    pin: String,
    owner: String,
}
pub async fn serve(
    home: &Path,
    state: GatewayStore,
    blobs: Arc<Mutex<ab_blobs::Store>>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let path = home.join("run/blobs-lab.sock");
    if path.try_exists()? {
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let uid = std::fs::metadata(&path)?.uid();
    let app = Arc::new(App {
        state,
        blobs,
        snapshots: Mutex::default(),
        serial: Mutex::new(()),
    });
    let router = Router::new()
        .route("/", axum::routing::post(handle))
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .with_state(app.clone());
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept(), if connections.len() < 4 => {
                let (socket, _) = accepted?;
                if socket.peer_cred()?.uid() != uid { continue; }
                let router = router.clone();
                connections.spawn(async move {
                    let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                        router.clone().oneshot(request.map(Body::new))
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(socket),service).await;
                });
            }
        }
    }
    connections.abort_all();
    for id in app.snapshots.lock().await.values() {
        app.state.with(|state| state.close_snapshot(id))?;
    }
    std::fs::remove_file(path)?;
    Ok(())
}
async fn handle(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Ok(request) = serde_json::from_slice::<Request>(&body) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid\n").into_response();
    };
    let _serial = app.serial.lock().await;
    match action(&app, request).await {
        Ok(value) => axum::Json(value).into_response(),
        Err(_) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "operation_failed\n",
        )
            .into_response(),
    }
}
async fn stage(app: &App, bytes: Vec<u8>) -> Result<(ab_blobs::Digest, String), Error> {
    let mut blobs = app.blobs.clone().lock_owned().await;
    tokio::task::spawn_blocking(move || blobs.stage(&bytes))
        .await
        .map_err(|_| Error::Invalid)?
        .map_err(|_| Error::Invalid)
}
async fn action(app: &App, request: Request) -> Result<Value, Error> {
    match request.action.as_str() {
        "stage" => {
            let (digest, pin) = stage(app, b"[ab-test] pinned blob".to_vec()).await?;
            return Ok(json!({"digest":digest,"pin":pin}));
        }
        "release" => app
            .blobs
            .lock()
            .await
            .release_pin(&request.pin)
            .map_err(|_| Error::Invalid)?,
        "snapshot" => {
            let visibility = ab_trust::Visibility {
                all: true,
                commands_all: true,
                ..Default::default()
            }
            .to_map();
            let snapshot = app
                .state
                .with(|state| state.freeze_snapshot(&visibility, 1))?;
            let old = app
                .snapshots
                .lock()
                .await
                .insert(request.owner, snapshot.id);
            if let Some(old) = old {
                app.state.with(|state| state.close_snapshot(&old))?;
            }
        }
        "snapshot-release" => {
            if let Some(id) = app.snapshots.lock().await.remove(&request.owner) {
                app.state.with(|state| state.close_snapshot(&id))?;
            }
        }
        "relay-fixture" => {
            let bytes = b"[ab-test] relay blob\n".repeat(8192);
            let size = bytes.len();
            let (digest, pin) = stage(app, bytes).await?;
            let capture = ab_wire::messages::Capture {
                source: Some(ab_trust::domain::Source::System),
                generation: format!("relay-{}", jiff::Timestamp::now()),
                checkpoint: "1".into(),
                observations: Some(vec![ab_wire::messages::Observation {
                    kind: "upsert".into(),
                    collection: "system.source".into(),
                    native_key: "relay-blob".into(),
                    data: Some(serde_json::value::to_raw_value(
                        &json!({"title":"[ab-test] relay blob","blob":{"digest":digest,"size":size,"state":"available"}}),
                    )?),
                    observed_at: ab_wire::gojson::GoTime::parse(
                        &jiff::Timestamp::now().to_string(),
                    )
                    .map_err(|_| Error::Invalid)?,
                    ..Default::default()
                }]),
                ..Default::default()
            };
            let committed = app.state.with(|state| state.commit_capture(&capture));
            app.blobs
                .lock()
                .await
                .release_pin(&pin)
                .map_err(|_| Error::Invalid)?;
            committed?;
            return Ok(json!({"digest":digest,"size":size,"title":"[ab-test] relay blob"}));
        }
        _ => return Err(Error::Invalid),
    }
    Ok(json!({"ok":true}))
}

pub async fn capture_boundary(home: &Path) -> Result<(), Error> {
    if std::fs::remove_file(home.join("lab-command-capture-armed")).is_ok() {
        ab_trust::atomic_write(
            &home.join("lab-command-capture-reached"),
            b"committed_before_receipt",
        )?;
        let path = home.join("lab-command-capture-release");
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut gate = std::fs::File::open(path)?;
            gate.read_exact(&mut [0u8; 1])
        })
        .await
        .map_err(|_| Error::Invalid)??;
    }
    if std::fs::remove_file(home.join("lab-capture-armed")).is_ok() {
        ab_trust::atomic_write(
            &home.join("lab-capture-reached"),
            b"capture_committed_before_acknowledgment",
        )?;
        std::future::pending::<()>().await;
    }
    Ok(())
}
