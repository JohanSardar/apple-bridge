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
    let result = tokio::task::spawn_blocking(move || {
        delivery.handle(uri.path(), &request, jiff::Timestamp::now().as_second())
    })
    .await;
    match result {
        Ok(Ok(value)) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            format!("{}\n", value.get()),
        )
            .into_response(),
        Ok(Err(Error::Authentication)) => {
            (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response()
        }
        Ok(Err(Error::NotFound)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(_)) => (StatusCode::BAD_REQUEST, "rejected\n").into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
