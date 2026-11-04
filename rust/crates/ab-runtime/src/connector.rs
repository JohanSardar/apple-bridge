use crate::Error;
use ab_frontdoor::web_connector::Connectors;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::Arc,
};

pub fn bind(home: &Path) -> Result<tokio::net::UnixListener, Error> {
    let path = home.join("run/connectors.sock");
    if path.try_exists()? {
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
#[derive(Deserialize)]
struct Decision {
    id: String,
    code: String,
    approve: bool,
}
async fn pending(State(connectors): State<Arc<Connectors>>) -> Result<Json<Value>, StatusCode> {
    connectors
        .pending()
        .and_then(|p| serde_json::to_value(p).map_err(|_| ab_trust::Error::Unavailable))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}
async fn decide(
    State(connectors): State<Arc<Connectors>>,
    Json(input): Json<Decision>,
) -> Result<Json<Value>, StatusCode> {
    connectors
        .decide(&input.id, &input.code, input.approve)
        .map(|()| Json(json!({"accepted":true})))
        .map_err(|_| StatusCode::BAD_REQUEST)
}
pub async fn serve(
    listener: tokio::net::UnixListener,
    connectors: Arc<Connectors>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Error> {
    let path = listener
        .local_addr()?
        .as_pathname()
        .ok_or(Error::Invalid)?
        .to_owned();
    let uid = std::fs::metadata(&path)?.uid();
    let router = Router::new()
        .route("/pending", get(pending))
        .route("/decision", post(decide))
        .with_state(connectors);
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept(), if tasks.len() < 16 => {
                let (stream, _) = accepted?;
                if stream.peer_cred()?.uid() != uid { continue; }
                let service = hyper_util::service::TowerToHyperService::new(router.clone());
                tasks.spawn(async move {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service)).await;
                });
            }
        }
    }
    tasks.abort_all();
    std::fs::remove_file(path)?;
    Ok(())
}
pub fn approval_window(
    helper_app: String,
    home: std::path::PathBuf,
) -> Option<Arc<dyn Fn() + Send + Sync>> {
    if !cfg!(target_os = "macos") || helper_app.is_empty() {
        return None;
    }
    let open = Arc::new(std::sync::atomic::AtomicBool::new(false));
    Some(Arc::new(move || {
        if open.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let bridge = std::env::current_exe().unwrap_or_default();
        let (helper_app, home, open) = (helper_app.clone(), home.clone(), open.clone());
        std::thread::spawn(move || {
            let status = std::process::Command::new("open")
                .args(["-n", "-W", "-g", "-a"])
                .arg(&helper_app)
                .args(["--args", "connector-approvals"])
                .arg(&home)
                .arg(&bridge)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if status.is_err() {
                eprintln!("connector approval window could not open");
            }
            open.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }))
}
pub async fn command(home: &Path, action: &str, id: &str, code: &str) -> Result<Value, Error> {
    let client = reqwest::Client::builder()
        .unix_socket(home.join("run/connectors.sock"))
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|_| Error::Invalid)?;
    let request = match action {
        "pending" => client.get("http://local/pending"),
        "allow" | "deny" => client
            .post("http://local/decision")
            .header("Content-Type", "application/json")
            .body(json!({"id":id,"code":code,"approve":action == "allow"}).to_string()),
        _ => return Err(Error::Invalid),
    };
    let response = request.send().await.map_err(|_| Error::Invalid)?;
    if !response.status().is_success() {
        return Err(Error::Invalid);
    }
    serde_json::from_slice(&response.bytes().await.map_err(|_| Error::Invalid)?).map_err(Into::into)
}
