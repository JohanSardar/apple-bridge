mod support;

use axum::{
    body::{Body, Bytes},
    http::{Request, StatusCode},
};
use futures_util::stream;
use sha2::{Digest, Sha256};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::{Check, world};
use tower::ServiceExt;

const CAPTURE: &str = "/v1/iphone/capture";
const HEADER: &str = "x-apple-bridge-upload";

fn counted(chunks: usize, size: usize) -> (Body, Arc<AtomicUsize>) {
    let read = Arc::new(AtomicUsize::new(0));
    let counter = read.clone();
    let body = Body::from_stream(stream::unfold(0, move |index| {
        let counter = counter.clone();
        async move {
            (index < chunks).then(|| {
                counter.fetch_add(size, Ordering::SeqCst);
                (
                    Ok::<_, Infallible>(Bytes::from(vec![b'x'; size])),
                    index + 1,
                )
            })
        }
    }));
    (body, read)
}

fn stalled() -> Body {
    Body::from_stream(stream::once(async {
        tokio::time::sleep(Duration::from_hours(24)).await;
        Ok::<_, Infallible>(Bytes::new())
    }))
}

fn post(path: &str, body: Body) -> Result<Request<Body>, axum::http::Error> {
    Request::post(path).body(body)
}

#[tokio::test]
async fn a_forged_header_is_refused_before_the_body_is_read() -> Check {
    let world = world()?;
    let router = world.delivery.router();
    let (body, read) = counted(64, 1 << 16);
    let request = Request::post(CAPTURE)
        .header(
            HEADER,
            world.phone.upload_header_for(CAPTURE, 0, 4096, &[0; 32]),
        )
        .body(body)?;
    let response = router.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        read.load(Ordering::SeqCst),
        0,
        "no body bytes before authentication"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_device_header_is_refused_before_the_body_is_read() -> Check {
    let world = world()?;
    let router = world.delivery.router();
    let (body, read) = counted(64, 1 << 16);
    let header = format!(
        "iphone:missing.{}.4096.{}=.{}==",
        ab_trust::decision_now().as_second(),
        "A".repeat(43),
        "A".repeat(86)
    );
    let request = Request::post(CAPTURE).header(HEADER, header).body(body)?;
    assert_eq!(
        router.oneshot(request).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(read.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn an_authenticated_header_admits_the_signed_body_only() -> Check {
    let world = world()?;
    let payload = serde_json::to_vec(&serde_json::json!({
        "command_id": "missing", "request_hash": "h", "content_type": "image/jpeg", "data": "eA=="
    }))?;
    let body = world.phone.body(CAPTURE, &payload)?;
    let header = world.phone.upload_header(CAPTURE, &body);

    let response = world
        .delivery
        .router()
        .oneshot(
            Request::post(CAPTURE)
                .header(HEADER, &header)
                .header("content-length", body.len())
                .body(Body::from(body.clone()))?,
        )
        .await?;
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let mut other = body.clone();
    let last = other.len() - 2;
    other[last] ^= 1;
    let response = world
        .delivery
        .router()
        .oneshot(
            Request::post(CAPTURE)
                .header(HEADER, &header)
                .header("content-length", other.len())
                .body(Body::from(other))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "body must match the signed digest"
    );
    Ok(())
}

#[tokio::test]
async fn a_header_declaring_a_different_length_is_refused_before_reading() -> Check {
    let world = world()?;
    let (body, read) = counted(4, 1 << 16);
    let digest = Sha256::digest(b"other");
    let header =
        world
            .phone
            .upload_header_for(CAPTURE, ab_trust::decision_now().as_second(), 5, &digest);
    let request = Request::post(CAPTURE)
        .header(HEADER, header)
        .header("content-length", 4usize << 16)
        .body(body)?;
    let response = world.delivery.router().oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(read.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn an_oversized_declared_body_is_refused_without_reading() -> Check {
    let world = world()?;
    let (body, read) = counted(4, 1 << 16);
    let request = Request::post(CAPTURE)
        .header("content-length", (24usize << 20) + 1)
        .body(body)?;
    let response = world.delivery.router().oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(read.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn unauthenticated_uploads_have_a_low_concurrency_limit() -> Check {
    let world = world()?;
    let router = world.delivery.router();
    let mut held = Vec::new();
    for _ in 0..2 {
        let router = router.clone();
        let request = post(CAPTURE, stalled())?;
        held.push(tokio::spawn(async move { router.oneshot(request).await }));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        router.clone().oneshot(post(CAPTURE, stalled())?),
    )
    .await??;
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    for task in held {
        task.abort();
    }
    Ok(())
}

#[tokio::test]
async fn an_old_app_without_the_header_can_still_upload() -> Check {
    let world = world()?;
    let payload = serde_json::to_vec(&serde_json::json!({
        "stream": "steps", "sequence": 1, "samples": [], "deleted": []
    }))?;
    let path = "/v1/iphone/health";
    let body = world.phone.body(path, &payload)?;
    let response = world
        .delivery
        .router()
        .oneshot(post(path, Body::from(body))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_stalled_upload_is_dropped_at_the_read_deadline() -> Check {
    let world = world()?;
    let router = world.delivery.router();
    let response = tokio::time::timeout(
        Duration::from_secs(600),
        router.oneshot(post(CAPTURE, stalled())?),
    )
    .await??;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    Ok(())
}

#[tokio::test]
async fn ordinary_routes_keep_their_small_limit() -> Check {
    let world = world()?;
    let response = world
        .delivery
        .router()
        .oneshot(post("/v1/iphone/pending", Body::from(vec![b'x'; 40_000]))?)
        .await?;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
}
