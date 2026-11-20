use crate::Error;
use ab_frontdoor::web_connector::Connectors;
use axum::{
    Json, Router,
    extract::{Extension, State},
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
#[derive(Deserialize)]
struct Ask {
    name: String,
}
#[derive(Deserialize)]
struct Answer {
    id: String,
    secret: String,
}
#[derive(Deserialize)]
struct TailnetAnswer {
    id: String,
    secret: String,
    resource: String,
}
async fn answer_tailnet(
    State(connectors): State<Arc<Connectors>>,
    Json(input): Json<TailnetAnswer>,
) -> Result<Json<Value>, StatusCode> {
    connectors
        .agent_answer_at(&input.id, &input.secret, Some(&input.resource))
        .and_then(|answer| serde_json::to_value(answer).map_err(|_| ab_trust::Error::Unavailable))
        .map(Json)
        .map_err(|_| StatusCode::GONE)
}
async fn ask(
    State(connectors): State<Arc<Connectors>>,
    Json(input): Json<Ask>,
) -> Result<Json<Value>, StatusCode> {
    connectors
        .ask_for_agent(&input.name, "this_mac", None)
        .and_then(|ticket| serde_json::to_value(ticket).map_err(|_| ab_trust::Error::Unavailable))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}
async fn ask_tailnet(
    State(connectors): State<Arc<Connectors>>,
    Extension(home): Extension<std::path::PathBuf>,
    Json(input): Json<Ask>,
) -> Result<Json<Value>, StatusCode> {
    let status = ab_remote::state::status(&home).map_err(|_| StatusCode::BAD_REQUEST)?;
    let route = status
        .routes
        .get(ab_remote::state::ROUTE)
        .filter(|route| route.state == "on" && route.url.starts_with("https://"))
        .ok_or(StatusCode::CONFLICT)?;
    let resource = format!("{}/mcp", route.url.trim_end_matches('/'));
    connectors
        .ask_for_agent(&input.name, "this_mac", Some(resource))
        .and_then(|ticket| serde_json::to_value(ticket).map_err(|_| ab_trust::Error::Unavailable))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn answer(
    State(connectors): State<Arc<Connectors>>,
    Json(input): Json<Answer>,
) -> Result<Json<Value>, StatusCode> {
    connectors
        .agent_answer(&input.id, &input.secret)
        .and_then(|answer| serde_json::to_value(answer).map_err(|_| ab_trust::Error::Unavailable))
        .map(Json)
        .map_err(|_| StatusCode::GONE)
}
pub async fn ask_for_agent(home: &Path, name: &str) -> Result<Value, ClientError> {
    exchange(home, "ask", Some(json!({"name": name}))).await
}
pub async fn ask_for_tailnet_agent(home: &Path, name: &str) -> Result<Value, ClientError> {
    exchange(home, "ask-tailnet", Some(json!({"name": name}))).await
}
pub async fn agent_answer(home: &Path, id: &str, secret: &str) -> Result<Value, ClientError> {
    exchange(home, "answer", Some(json!({"id": id, "secret": secret}))).await
}
pub async fn tailnet_agent_answer(
    home: &Path,
    id: &str,
    secret: &str,
    resource: &str,
) -> Result<Value, ClientError> {
    exchange(
        home,
        "answer-tailnet",
        Some(json!({"id": id, "secret": secret, "resource": resource})),
    )
    .await
}
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid connector action")]
    InvalidAction,
    #[error("connector transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("connector rejected the request with {0}")]
    Rejected(reqwest::StatusCode),
    #[error("invalid connector response: {0}")]
    Decode(#[from] serde_json::Error),
}

async fn exchange(home: &Path, route: &str, body: Option<Value>) -> Result<Value, ClientError> {
    let client = reqwest::Client::builder()
        .unix_socket(home.join("run/connectors.sock"))
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let url = format!("http://local/{route}");
    let request = if let Some(body) = body {
        client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
    } else {
        client.get(url)
    };
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(ClientError::Rejected(response.status()));
    }
    Ok(serde_json::from_slice(&response.bytes().await?)?)
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
    let home = path
        .parent()
        .and_then(Path::parent)
        .ok_or(Error::Invalid)?
        .to_owned();
    let router = Router::new()
        .route("/pending", get(pending))
        .route("/decision", post(decide))
        .route("/ask", post(ask))
        .route("/ask-tailnet", post(ask_tailnet))
        .route("/answer", post(answer))
        .route("/answer-tailnet", post(answer_tailnet))
        .layer(Extension(home))
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
pub async fn command(
    home: &Path,
    action: &str,
    id: &str,
    code: &str,
) -> Result<Value, ClientError> {
    match action {
        "pending" => exchange(home, "pending", None).await,
        "allow" | "deny" => {
            exchange(
                home,
                "decision",
                Some(json!({"id": id, "code": code, "approve": action == "allow"})),
            )
            .await
        }
        _ => Err(ClientError::InvalidAction),
    }
}
