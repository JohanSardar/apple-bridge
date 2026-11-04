use ab_trust::{Authority, Error, Principal};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Debug)]
struct Gateway {
    url: String,
    client: reqwest::blocking::Client,
}
impl Gateway {
    fn call(&self, path: &str, input: &Value) -> Result<Value, Error> {
        Ok(self
            .client
            .post(format!("{}{path}", self.url))
            .json(input)
            .send()?
            .error_for_status()?
            .json()?)
    }
}
impl ab_frontdoor::web::Approvals for Gateway {
    fn pending(&self) -> Result<Vec<Value>, Error> {
        Ok(serde_json::from_value(
            self.call("/pending", &Value::Null)?,
        )?)
    }
    fn command(&self, id: &str) -> Result<Value, Error> {
        self.call("/command", &json!({"id":id}))
    }
    fn decide(&self, owner: &Principal, id: &str, approve: bool) -> Result<Value, Error> {
        self.call(
            "/decision",
            &json!({"id":id,"owner":owner,"approve":approve}),
        )
    }
}
async fn proxy(State(upstream): State<String>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, 1_048_576).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let client = reqwest::Client::new();
    let mut request = client
        .request(parts.method, format!("{upstream}{}", parts.uri))
        .body(body);
    for (key, value) in &parts.headers {
        if key != axum::http::header::CONNECTION && key != axum::http::header::CONTENT_LENGTH {
            request = request.header(key, value);
        }
    }
    let Ok(response) = request.send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let status = response.status();
    let mut headers = response.headers().clone();
    headers.remove(axum::http::header::TRANSFER_ENCODING);
    headers.remove(axum::http::header::CONTENT_LENGTH);
    let Ok(bytes) = response.bytes().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("isolated home")?;
    let address = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:7780".into());
    let upstream = std::env::args().nth(3);
    let backend = std::env::args().nth(4);
    let authority = Arc::new(Authority::open(home)?);
    let enroll = authority.create_web_enroll()?;
    let (token, _) = authority.create_device_session("trusted")?;
    let approvals: Option<Arc<dyn ab_frontdoor::web::Approvals>> = if let Some(url) = backend {
        Some(
            tokio::task::spawn_blocking(move || {
                reqwest::blocking::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map(|client| {
                        Arc::new(Gateway { url, client }) as Arc<dyn ab_frontdoor::web::Approvals>
                    })
            })
            .await??,
        )
    } else {
        None
    };
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    let connectors = ab_frontdoor::web_connector::Connectors::new(authority.clone());
    let web = ab_frontdoor::web::Web::new(authority, address, String::new(), approvals, connectors);
    let mut router = web.router();
    if let Some(url) = upstream {
        router = router.merge(
            axum::Router::new()
                .route("/v1/{*path}", any(proxy))
                .route("/mcp", any(proxy))
                .with_state(url),
        );
    }
    println!(
        "{}",
        json!({"address":address.to_string(),"enroll":enroll,"token":token})
    );
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
