#![allow(clippy::unwrap_used, clippy::expect_used)]

use ab_relay::{
    http::Api,
    server::Server,
    store::{Limits, Store},
};
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower::ServiceExt;

fn router(dir: &tempfile::TempDir) -> Router {
    let store = Store::open(
        dir.path().join("relay.json"),
        "relay.test",
        Limits {
            connections: 8,
            bytes_per_second: 1 << 24,
            monthly_bytes: 1 << 34,
        },
    )
    .unwrap();
    let server = Arc::new(Server::new(store, Duration::from_secs(5)));
    Arc::new(Api::new(server, "admin-token".into()).unwrap()).router()
}

async fn send(router: &Router, method: &str, path: &str, source: &str, body: &Value) -> (StatusCode, Value) {
    let address: SocketAddr = format!("{source}:1").parse().unwrap();
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer admin-token")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(address));
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn unauthenticated_proof_routes_have_a_budget_per_source() {
    for (path, body) in [
        ("/dns", json!({"value": "x"})),
        ("/domain", json!({"domain": "mac.example.test"})),
        ("/certificate", json!({"fingerprint": "0".repeat(64)})),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let router = router(&dir);
        let mut throttled_after = None;
        for attempt in 0..200 {
            let (status, _) = send(&router, "POST", path, "203.0.113.5", &body).await;
            if status == StatusCode::TOO_MANY_REQUESTS {
                throttled_after = Some(attempt);
                break;
            }
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
        assert_eq!(throttled_after, Some(60), "{path} has no budget");
        assert_eq!(
            send(&router, "POST", path, "203.0.113.6", &body).await.0,
            StatusCode::FORBIDDEN,
            "{path} refused a different source"
        );
    }
}

#[tokio::test]
async fn an_ipv6_host_cannot_mint_budgets_by_changing_its_interface_id() {
    let dir = tempfile::tempdir().unwrap();
    let router = router(&dir);
    let body = json!({"value": "x"});
    let mut last = StatusCode::OK;
    for interface in 0..61 {
        last = send(
            &router,
            "POST",
            "/dns",
            &format!("[2001:db8:0:1::{interface:x}]"),
            &body,
        )
        .await
        .0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_flood_of_challenges_cannot_stop_a_mac_getting_a_usable_one() {
    let dir = tempfile::tempdir().unwrap();
    let router = router(&dir);
    let mut issued = 0;
    'flood: for high in 0..=255u8 {
        for low in 0..=255u8 {
            let source = format!("198.51.{high}.{low}");
            for _ in 0..30 {
                let (status, _) = send(&router, "GET", "/challenge", &source, &Value::Null).await;
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "challenge {issued} refused while the pool was flooded"
                );
                issued += 1;
                if issued >= 4100 {
                    break 'flood;
                }
            }
        }
    }
    let (status, value) = send(&router, "GET", "/challenge", "203.0.113.9", &Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(value["nonce"].as_str().is_some_and(|nonce| !nonce.is_empty()));
}

#[tokio::test]
async fn a_late_policy_write_cannot_undo_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let router = router(&dir);
    let (status, admitted) = send(
        &router,
        "POST",
        "/admit",
        "127.0.0.1",
        &json!({"public_key": "A".repeat(43)}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{admitted}");
    let hostname = admitted["hostname"].as_str().unwrap().to_owned();
    let post = |body: Value| {
        let router = router.clone();
        async move { send(&router, "POST", "/policy", "127.0.0.1", &body).await.0 }
    };
    assert_eq!(
        post(json!({"hostname": hostname, "disabled": true, "version": 7})).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(json!({"hostname": hostname, "disabled": false, "version": 6})).await,
        StatusCode::NO_CONTENT,
        "a stale write is acknowledged so the sender stops retrying it"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("relay.json")).unwrap()).unwrap();
    assert_eq!(state[&hostname]["disabled"], true, "the stale write won");
    assert_eq!(
        post(json!({"hostname": hostname, "disabled": false})).await,
        StatusCode::NO_CONTENT,
        "a control service from before this release sends no version and is still obeyed"
    );
}
