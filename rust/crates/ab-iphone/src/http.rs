use crate::{
    Error,
    delivery::{Delivery, Gateway},
};
use ab_trust::iphone::{Request, UploadAuthorization};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, Uri, header::CONTENT_LENGTH},
    response::IntoResponse,
    routing::post,
};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, SemaphorePermit};

const UPLOAD_HEADER: &str = "x-apple-bridge-upload";
const UPLOAD_HEADER_LIMIT: usize = 512;
const READ_DEADLINE: Duration = Duration::from_secs(120);
const SLOT_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) struct Admission {
    verifying: Semaphore,
    anonymous: Semaphore,
    authenticated: Semaphore,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            verifying: Semaphore::new(8),
            anonymous: Semaphore::new(2),
            authenticated: Semaphore::new(2),
        }
    }
}

impl<G: Gateway + 'static> Delivery<G> {
    pub fn router(self: &Arc<Self>) -> Router {
        let ordinary = Router::new()
            .route("/v1/iphone/pending", post(handle::<G>))
            .route("/v1/iphone/result", post(handle::<G>))
            .route("/v1/iphone/push-token", post(handle::<G>))
            .route("/v1/iphone/live-token", post(handle::<G>))
            .route("/v1/iphone/live-disable", post(handle::<G>))
            .route("/v1/iphone/location-records", post(handle::<G>))
            .route("/v1/iphone/approval", post(handle::<G>))
            .layer(DefaultBodyLimit::max(32768));
        ordinary
            .merge(
                Router::new()
                    .route("/v1/iphone/capture", post(upload::<G>))
                    .route("/v1/iphone/health", post(upload::<G>))
                    .layer(DefaultBodyLimit::disable()),
            )
            .with_state(self.clone())
    }
}
fn upload_limit(path: &str) -> usize {
    if path == "/v1/iphone/capture" {
        24 << 20
    } else {
        16 << 20
    }
}
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    headers.get(CONTENT_LENGTH)?.to_str().ok()?.parse().ok()
}
async fn authenticate<G: Gateway + 'static>(
    delivery: &Arc<Delivery<G>>,
    path: &str,
    header: &axum::http::HeaderValue,
) -> Result<UploadAuthorization, StatusCode> {
    let Ok(_slot) = delivery.uploads.verifying.try_acquire() else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    if header.len() > UPLOAD_HEADER_LIMIT {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let header = header
        .to_str()
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .to_owned();
    let (delivery, path) = (delivery.clone(), path.to_owned());
    tokio::task::spawn_blocking(move || delivery.authority.verify_iphone_upload(&path, &header))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::UNAUTHORIZED)
}
async fn upload_slot<G: Gateway>(
    delivery: &Delivery<G>,
    authenticated: bool,
) -> Result<SemaphorePermit<'_>, StatusCode> {
    if authenticated {
        tokio::time::timeout(SLOT_WAIT, delivery.uploads.authenticated.acquire())
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    } else {
        delivery
            .uploads
            .anonymous
            .try_acquire()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
}
async fn upload<G: Gateway + 'static>(
    State(delivery): State<Arc<Delivery<G>>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> axum::response::Response {
    let path = uri.path();
    let mut limit = upload_limit(path);
    let declared = declared_length(&headers);
    if declared.is_some_and(|length| length > limit as u64) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let authorization = match headers.get(UPLOAD_HEADER) {
        Some(header) => match authenticate(&delivery, path, header).await {
            Ok(authorization) => Some(authorization),
            Err(status) => return (status, "unauthorized\n").into_response(),
        },
        None => None,
    };
    if let Some(authorization) = &authorization {
        if declared.is_some_and(|length| length != authorization.length) {
            return (StatusCode::BAD_REQUEST, "rejected\n").into_response();
        }
        let Ok(length) = usize::try_from(authorization.length) else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        if length > limit {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        limit = length;
    }
    let _slot = match upload_slot(&delivery, authorization.is_some()).await {
        Ok(slot) => slot,
        Err(status) => return status.into_response(),
    };
    let bytes = match tokio::time::timeout(READ_DEADLINE, axum::body::to_bytes(body, limit)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    if authorization.is_some_and(|authorization| {
        authorization.digest.as_slice() != Sha256::digest(&bytes).as_slice()
    }) {
        return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
    }
    respond(delivery.clone(), &uri, &bytes).await
}
async fn handle<G: Gateway + 'static>(
    State(delivery): State<Arc<Delivery<G>>>,
    uri: Uri,
    body: Bytes,
) -> axum::response::Response {
    respond(delivery, &uri, &body).await
}
async fn respond<G: Gateway + 'static>(
    delivery: Arc<Delivery<G>>,
    uri: &Uri,
    body: &[u8],
) -> axum::response::Response {
    let Ok(request) = serde_json::from_slice::<Request>(body) else {
        return (StatusCode::BAD_REQUEST, "invalid\n").into_response();
    };
    #[cfg(feature = "lab-harness")]
    let observed = match observer::begin(uri.path(), &request).await {
        Ok(observed) => observed,
        Err(()) => {
            return (StatusCode::SERVICE_UNAVAILABLE, "observer unavailable\n").into_response();
        }
    };
    let path = uri.path().to_owned();
    let result = tokio::task::spawn_blocking(move || {
        delivery.handle(&path, &request, ab_trust::decision_now().as_second())
    })
    .await;
    let (status, json, text) = match result {
        Ok(Ok(value)) => (StatusCode::OK, true, format!("{}\n", value.get())),
        Ok(Err(Error::Authentication)) => {
            (StatusCode::UNAUTHORIZED, false, "unauthorized\n".into())
        }
        Ok(Err(Error::NotFound)) => (StatusCode::NOT_FOUND, false, String::new()),
        Ok(Err(_)) => (StatusCode::BAD_REQUEST, false, "rejected\n".into()),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, false, String::new()),
    };
    #[cfg(feature = "lab-harness")]
    observer::end(observed, status, &text).await;
    if json {
        (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            text,
        )
            .into_response()
    } else {
        (status, text).into_response()
    }
}

#[cfg(feature = "lab-harness")]
mod observer {
    use ab_trust::iphone::Request;
    use axum::http::StatusCode;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn send(socket: &str, value: &serde_json::Value) -> Result<i64, ()> {
        let mut stream = UnixStream::connect(socket).map_err(|_| ())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|_| ())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|_| ())?;
        let mut line = serde_json::to_vec(value).map_err(|_| ())?;
        line.push(b'\n');
        stream.write_all(&line).map_err(|_| ())?;
        let mut ack = String::new();
        BufReader::new(stream).read_line(&mut ack).map_err(|_| ())?;
        let ack: serde_json::Value = serde_json::from_str(&ack).map_err(|_| ())?;
        ack["sequence"].as_i64().ok_or(())
    }

    pub async fn begin(path: &str, request: &Request) -> Result<Option<(String, i64)>, ()> {
        let Ok(socket) = std::env::var("APPLE_BRIDGE_LAB_IPHONE_OBSERVER") else {
            return Ok(None);
        };
        if socket.is_empty() {
            return Ok(None);
        }
        let value = serde_json::json!({"event":"request","path":path,"device_id":request.device_id,"at":request.at,"payload":request.payload});
        let target = socket.clone();
        let sequence = tokio::task::spawn_blocking(move || send(&target, &value))
            .await
            .map_err(|_| ())??;
        Ok(Some((socket, sequence)))
    }

    pub async fn end(observed: Option<(String, i64)>, status: StatusCode, body: &str) {
        let Some((socket, sequence)) = observed else {
            return;
        };
        let value = serde_json::json!({"event":"response","request":sequence,"status":status.as_u16(),"body":STANDARD.encode(body)});
        let _ = tokio::task::spawn_blocking(move || send(&socket, &value)).await;
    }
}
