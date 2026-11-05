#![allow(clippy::unwrap_used, clippy::expect_used)]
use ab_frontdoor::{web::Web, web_connector::Connectors};
use ab_trust::Authority;
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

const ORIGIN: &str = "http://127.0.0.1:7780";
const HOST: &str = "127.0.0.1:7780";

struct Fixture {
    _home: tempfile::TempDir,
    router: Router,
    authority: Arc<Authority>,
}
fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let authority = Arc::new(Authority::open(home.path()).unwrap());
    let connectors = Connectors::new(authority.clone());
    let web = Web::new(
        authority.clone(),
        HOST.parse().unwrap(),
        String::new(),
        None,
        connectors,
    );
    Fixture {
        _home: home,
        router: web.router(),
        authority,
    }
}
impl Fixture {
    async fn post(&self, path: &str, body: &serde_json::Value, source: Option<&str>) -> StatusCode {
        let mut request = Request::post(path)
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        if let Some(source) = source {
            let address: SocketAddr = source.parse().unwrap();
            request.extensions_mut().insert(ConnectInfo(address));
        }
        self.router
            .clone()
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }
    async fn exhaust(&self, source: Option<&str>) -> usize {
        for attempt in 0..1000 {
            let status = self
                .post("/auth/login/options", &json!({}), source)
                .await;
            if status == StatusCode::TOO_MANY_REQUESTS {
                return attempt;
            }
        }
        panic!("the budget never ran out");
    }
}

#[tokio::test]
async fn a_flood_from_one_tunnel_client_does_not_refuse_another() {
    let f = fixture();
    let spent = f.exhaust(Some("203.0.113.1:0")).await;
    assert_eq!(spent, 10);
    assert_ne!(
        f.post("/auth/login/options", &json!({}), Some("203.0.113.2:0"))
            .await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_ne!(
        f.post("/auth/login/options", &json!({}), Some("[2001:db8:1::1]:0"))
            .await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn addresses_inside_one_ipv6_network_share_a_budget() {
    let f = fixture();
    f.exhaust(Some("[2001:db8:1:2::1]:0")).await;
    assert_eq!(
        f.post("/auth/login/options", &json!({}), Some("[2001:db8:1:2::9]:0"))
            .await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn clients_on_a_transport_with_no_source_get_a_wider_shared_budget() {
    let f = fixture();
    assert_eq!(f.exhaust(None).await, 60);
}

#[tokio::test]
async fn the_owner_can_recover_with_an_enrollment_link_while_login_is_flooded() {
    let f = fixture();
    f.exhaust(None).await;
    let token = f.authority.create_web_enroll().unwrap();
    assert_eq!(
        f.post("/auth/enroll/options", &json!({"token": token}), None)
            .await,
        StatusCode::OK
    );
    assert_eq!(
        f.post("/auth/enroll/options", &json!({"token": "wrong"}), None)
            .await,
        StatusCode::TOO_MANY_REQUESTS,
        "a guessed token gets no way around the budget"
    );
}
