use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderName, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use std::{net::SocketAddr, time::Duration};

const BODY_LIMIT: usize = 64 * 1024;

#[derive(Clone)]
struct Upstream {
    base: String,
    client: reqwest::Client,
}

pub fn router(upstream: &str) -> crate::Result<Router> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| crate::Error::Configuration)?;
    Ok(Router::new().fallback(any(proxy)).with_state(Upstream {
        base: format!("http://{upstream}"),
        client,
    }))
}

fn hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "x-forwarded-for"
    )
}

async fn proxy(
    State(upstream): State<Upstream>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, BODY_LIMIT).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", axum::http::uri::PathAndQuery::as_str);
    let mut outgoing = upstream
        .client
        .request(parts.method, format!("{}{path}", upstream.base))
        .header("x-forwarded-for", peer.ip().to_canonical().to_string())
        .body(body);
    for (name, value) in &parts.headers {
        if !hop_by_hop(name) {
            outgoing = outgoing.header(name, value);
        }
    }
    let Ok(reply) = outgoing.send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let status = reply.status();
    let headers = reply.headers().clone();
    let Ok(bytes) = reply.bytes().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    for (name, value) in &headers {
        if !hop_by_hop(name) && name != header::SERVER {
            response.headers_mut().append(name, value.clone());
        }
    }
    response
}
