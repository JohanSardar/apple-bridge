#![allow(clippy::unwrap_used)]
use ab_frontdoor::{Audit, AuditEntry, Config, Frontdoor};
use ab_operations::{
    Code, Command, CommandTarget, Cursor, Error, ForwardReply, GatewayRoute, GatewayTransport,
    Identity, Principal, ReadQuery, ReadStore, RemoteGateway, Service, Task, Visibility,
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use futures_util::StreamExt;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tower::ServiceExt;

#[derive(Debug)]
struct Expiring(Option<jiff::Timestamp>);
impl Identity for Expiring {
    fn authenticate(&self, _: &str) -> Result<Principal, Error> {
        Ok(Principal {
            id: "agent:synthetic".into(),
            kind: "agent".into(),
            grants: vec!["messages:read".into(), "messages:write".into()],
            level: "full".into(),
            resource: String::new(),
            remote: false,
            expires_at: self.0,
        })
    }
    fn current(&self, principal: &Principal) -> Result<Principal, Error> {
        Ok(principal.clone())
    }
    fn sign(&self, _: &Principal, _: &str, _: &str) -> Result<String, Error> {
        Ok("synthetic".into())
    }
}
#[derive(Debug)]
struct Quiet;
impl ReadStore for Quiet {
    fn blob<'a>(&'a self, _: &'a Visibility, record: &'a str, _: &'a str) -> Task<'a, Vec<u8>> {
        Box::pin(async move {
            match record {
                "corrupt" => Err(Code::Corrupt.into()),
                "deadline" => Err(Code::Deadline.into()),
                _ => Ok(b"fixture".to_vec()),
            }
        })
    }
    fn head(&self) -> Result<Cursor, Error> {
        Ok(Cursor {
            epoch: "synthetic".into(),
            subscription_revision: 1,
            sequence: 0,
        })
    }
    fn read(&self, _: &Visibility, _: &ReadQuery) -> Result<Box<RawValue>, Error> {
        Err(Code::Unavailable.into())
    }
    fn read_model(&self, _: &Visibility, _: &str, _: &Value) -> Result<Box<RawValue>, Error> {
        Err(Code::Unavailable.into())
    }
    fn changes(&self, _: &Visibility, _: &Cursor) -> Result<Option<Box<RawValue>>, Error> {
        Ok(None)
    }
}
#[derive(Debug)]
struct Plain;
impl ab_operations::Codec for Plain {
    fn json(&self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(raw.to_vec())
    }
    fn command_hash(&self, _: &Command) -> Result<String, Error> {
        Ok("hash".into())
    }
    fn fingerprint(&self, _: &[u8]) -> String {
        "hash".into()
    }
}
#[derive(Debug)]
struct Validating;
impl GatewayTransport for Validating {
    fn connected(&self) -> bool {
        true
    }
    fn exchange<'a>(
        &'a self,
        _: &'a GatewayRoute,
        _: String,
        body: Vec<u8>,
    ) -> Task<'a, ForwardReply> {
        Box::pin(async move {
            let request: Value = serde_json::from_slice(&body).unwrap();
            let command = serde_json::from_value(request["command"].clone()).unwrap();
            ab_operations::command_contract::validate(&command)?;
            Err(Code::Unavailable.into())
        })
    }
}
#[derive(Debug)]
struct NoAudit;
impl Audit for NoAudit {
    fn record(&self, _: AuditEntry) -> Result<(), Error> {
        Ok(())
    }
}

fn router(expires_at: Option<jiff::Timestamp>, requests_per_second: u32) -> Router {
    let identity: Arc<dyn Identity> = Arc::new(Expiring(expires_at));
    let store: Arc<dyn ReadStore> = Arc::new(Quiet);
    let codec: Arc<dyn ab_operations::Codec> = Arc::new(Plain);
    let service = Arc::new(Service {
        store: store.clone(),
        identity: identity.clone(),
        target: CommandTarget::Remote(Arc::new(RemoteGateway {
            route: GatewayRoute::Feed,
            transport: Arc::new(Validating),
            identity,
            store,
            codec: codec.clone(),
        })),
        codec,
    });
    Frontdoor::new(
        service,
        Config {
            hosts: vec!["bridge.test".into()],
            origins: vec![],
            public: false,
            requests_per_second,
        },
        Arc::new(NoAudit),
    )
    .router()
}

fn request(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "bridge.test")
        .header(header::AUTHORIZATION, "Bearer synthetic")
}

async fn stream_body(expires_at: Option<jiff::Timestamp>, within: Duration) -> Option<Vec<u8>> {
    let response = router(expires_at, 100)
        .oneshot(request("GET", "/v1/changes").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    tokio::time::timeout(within, to_bytes(response.into_body(), 1 << 20))
        .await
        .ok()
        .map(|bytes| bytes.unwrap().to_vec())
}

#[tokio::test]
async fn a_change_stream_closes_when_its_access_token_expires() {
    let soon = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(1))
        .unwrap();
    let body = stream_body(Some(soon), Duration::from_secs(8)).await;
    assert!(body.is_some(), "the stream kept running past token expiry");
}

#[tokio::test]
async fn a_change_stream_without_an_expiry_stays_open() {
    assert!(stream_body(None, Duration::from_secs(2)).await.is_none());
}

async fn error(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["error"].clone()
}

#[tokio::test]
async fn rate_limit_is_retryable_json_with_retry_after() {
    let response = router(None, 0)
        .oneshot(
            request("GET", "/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert_eq!(
        error(response, StatusCode::TOO_MANY_REQUESTS).await,
        json!({"code": "rate_limited", "message": "rate limited", "retryable": true})
    );
}

#[tokio::test]
async fn command_validation_returns_fields_in_the_rest_error() {
    let response = router(None, 100)
        .oneshot(
            request("POST", "/v1/op/messages.send")
                .body(Body::from(r#"{"args":{"text":42}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let problem = error(response, StatusCode::BAD_REQUEST).await;
    assert_eq!(problem["code"], "invalid");
    assert_eq!(problem["field"], "args.text");
    assert_eq!(problem["expected"], "string");
    assert_eq!(problem["hint"], "Correct the input before retrying.");
    assert_eq!(problem["retryable"], false);
    assert!(problem.get("current").is_none());
}

#[tokio::test]
async fn missing_scope_returns_forbidden_without_a_bearer_challenge() {
    let response = router(None, 100)
        .oneshot(
            request("POST", "/v1/op/notes.list")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    assert!(
        response.headers()["server-timing"]
            .to_str()
            .unwrap()
            .starts_with("readmodel;dur=")
    );
    assert_eq!(
        error(response, StatusCode::FORBIDDEN).await,
        json!({"code": "forbidden", "message": "missing required scope notes:read", "retryable": false})
    );
}

#[tokio::test]
async fn body_limits_return_json_and_uploads_can_exceed_the_default_extractor_limit() {
    for (path, chunks, status, code) in [
        (
            "/v1/op/notes.list",
            2,
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
        ),
        (
            "/v1/blobs",
            3,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        ),
        ("/v1/blobs", 101, StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
    ] {
        let stream =
            futures_util::stream::repeat(Ok::<_, Infallible>(Bytes::from(vec![0; 1 << 20])))
                .take(chunks);
        let response = router(None, 100)
            .oneshot(
                request("POST", path)
                    .body(Body::from_stream(stream))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(error(response, status).await["code"], code, "{path}");
    }
}

#[tokio::test]
async fn blob_failures_use_json_and_preserve_range_metadata() {
    for (path, range, status, code) in [
        (
            "/v1/blobs/digest",
            "bytes=0-1",
            StatusCode::BAD_REQUEST,
            "invalid",
        ),
        (
            "/v1/blobs/digest?record=fixture",
            "bytes=+1-2",
            StatusCode::RANGE_NOT_SATISFIABLE,
            "invalid_range",
        ),
        (
            "/v1/blobs/digest?record=corrupt",
            "bytes=0-1",
            StatusCode::INTERNAL_SERVER_ERROR,
            "corrupt",
        ),
        (
            "/v1/blobs/digest?record=deadline",
            "bytes=0-1",
            StatusCode::GATEWAY_TIMEOUT,
            "deadline",
        ),
    ] {
        let response = router(None, 100)
            .oneshot(
                request("GET", path)
                    .header(header::RANGE, range)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */7");
        }
        assert_eq!(error(response, status).await["code"], code, "{path}");
    }
}
