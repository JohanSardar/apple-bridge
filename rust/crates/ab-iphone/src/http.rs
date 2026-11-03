use crate::{
    Error,
    delivery::{Delivery, Gateway},
};
use ab_trust::iphone::Request;
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::post,
};
use std::sync::Arc;

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
                    .route("/v1/iphone/capture", post(handle::<G>))
                    .layer(DefaultBodyLimit::max(24 << 20)),
            )
            .merge(
                Router::new()
                    .route("/v1/iphone/health", post(handle::<G>))
                    .layer(DefaultBodyLimit::max(16 << 20)),
            )
            .with_state(self.clone())
    }
}
async fn handle<G: Gateway + 'static>(
    State(delivery): State<Arc<Delivery<G>>>,
    uri: Uri,
    body: Bytes,
) -> axum::response::Response {
    let Ok(request) = serde_json::from_slice::<Request>(&body) else {
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
