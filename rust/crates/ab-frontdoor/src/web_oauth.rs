use crate::web::{Web, field, fields, reject, respond};
use crate::web_pages;
use ab_trust::{
    Grant, Principal,
    devices::Session,
    oauth::{self, Client, Code, Exchange, OFFLINE_ACCESS},
};
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::collections::HashMap;

type Query = HashMap<String, Vec<String>>;

fn all_scopes() -> Vec<String> {
    Grant::agent_scopes()
}

pub(crate) fn protected_resource(origin: &str) -> Value {
    json!({
        "resource": format!("{origin}/mcp"),
        "authorization_servers": [origin],
        "scopes_supported": all_scopes(),
        "bearer_methods_supported": ["header"],
    })
}

pub(crate) fn authorization_server(origin: &str) -> Value {
    let mut scopes = all_scopes();
    scopes.push(OFFLINE_ACCESS.to_owned());
    json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": scopes,
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    })
}

pub(crate) fn register(web: &Web, body: &[u8]) -> Response {
    serde_json::from_slice::<Client>(body)
        .ok()
        .and_then(|client| web.authority.register_oauth(client).ok())
        .map_or_else(
            || respond(400, &json!({"error": "invalid_client_metadata"})),
            |client| respond(201, &client),
        )
}

fn resource_of(given: &str, origin: &str) -> Option<String> {
    let expected = format!("{origin}/mcp");
    (given.is_empty() || given == expected).then_some(expected)
}

pub(crate) fn token(web: &Web, body: &[u8], origin: &str) -> Response {
    let form = fields(&String::from_utf8_lossy(body));
    if form.is_empty() || form.values().any(|values| values.len() != 1) {
        return respond(400, &json!({"error": "invalid_request"}));
    }
    if !["authorization_code", "refresh_token"].contains(&field(&form, "grant_type")) {
        return respond(400, &json!({"error": "unsupported_grant_type"}));
    }
    let Some(resource) = resource_of(field(&form, "resource"), origin) else {
        return respond(400, &json!({"error": "invalid_target"}));
    };
    let exchange = Exchange {
        grant_type: field(&form, "grant_type").into(),
        client_id: field(&form, "client_id").into(),
        code: field(&form, "code").into(),
        redirect_uri: field(&form, "redirect_uri").into(),
        code_verifier: field(&form, "code_verifier").into(),
        resource,
        refresh_token: field(&form, "refresh_token").into(),
        scope: field(&form, "scope").into(),
    };
    web.authority.exchange_oauth(&exchange).map_or_else(
        |_| respond(400, &json!({"error": "invalid_grant"})),
        |tokens| respond(200, &tokens),
    )
}

fn requested_scopes(query: &Query) -> Vec<Grant> {
    let raw = field(query, "scope");
    if raw.trim().is_empty() {
        return Grant::parse("messages:read").into_iter().collect();
    }
    raw.split_whitespace()
        .filter(|scope| *scope != OFFLINE_ACCESS)
        .filter_map(|scope| Grant::parse(scope).ok())
        .filter(|grant| oauth::agent_grants("approve", std::slice::from_ref(grant)).is_ok())
        .collect()
}

fn resolve_client(web: &Web, query: &Query) -> Result<Client, ab_trust::Error> {
    if field(query, "client_id").starts_with("https://") {
        web.metadata
            .fetch(field(query, "client_id"), field(query, "redirect_uri"))
    } else {
        web.authority
            .oauth_client(field(query, "client_id"), field(query, "redirect_uri"))
    }
}

fn client_redirect(
    redirect_uri: &str,
    state: &str,
    origin: &str,
    result: &[(&str, &str)],
) -> Option<String> {
    let mut destination = url::Url::parse(redirect_uri).ok()?;
    destination
        .query_pairs_mut()
        .append_pair("state", state)
        .append_pair("iss", origin)
        .extend_pairs(result);
    Some(destination.into())
}

fn see_other(location: &str) -> Response {
    match location.parse::<header::HeaderValue>() {
        Ok(value) => (StatusCode::SEE_OTHER, [(header::LOCATION, value)]).into_response(),
        Err(_) => web_pages::invalid_request(),
    }
}

fn request_error(query: &Query, origin: &str) -> Option<&'static str> {
    if field(query, "response_type") != "code" {
        return Some("unsupported_response_type");
    }
    if resource_of(field(query, "resource"), origin).is_none() {
        return Some("invalid_target");
    }
    if field(query, "state").len() > 2048
        || field(query, "code_challenge_method") != "S256"
        || !URL_SAFE_NO_PAD
            .decode(field(query, "code_challenge"))
            .is_ok_and(|bytes| bytes.len() == 32)
    {
        return Some("invalid_request");
    }
    requested_scopes(query)
        .is_empty()
        .then_some("invalid_scope")
}

fn code_for(client: &Client, query: &Query, origin: &str) -> Code {
    Code {
        client_id: client.client_id.clone(),
        redirect_uri: field(query, "redirect_uri").into(),
        challenge: field(query, "code_challenge").into(),
        resource: format!("{origin}/mcp"),
        ..Code::default()
    }
}

pub(crate) fn authorize(
    web: &Web,
    request: &axum::http::request::Parts,
    body: &[u8],
    origin: &str,
    query: &Query,
    identity: Option<&(Principal, Session)>,
) -> Response {
    if query.values().any(|values| values.len() != 1) {
        return web_pages::invalid_request();
    }
    let Ok(client) = resolve_client(web, query) else {
        return web_pages::invalid_request();
    };
    let redirect_uri = field(query, "redirect_uri");
    let state = field(query, "state");
    if let Some(error) = request_error(query, origin) {
        return client_redirect(redirect_uri, state, origin, &[("error", error)])
            .map_or_else(web_pages::invalid_request, |to| see_other(&to));
    }
    if request.method == "GET" {
        return show_consent(web, &client, query, origin, identity.is_some());
    }
    decide(web, request, body, origin, query, &client)
}

fn show_consent(
    web: &Web,
    client: &Client,
    query: &Query,
    origin: &str,
    signed_in: bool,
) -> Response {
    let redirect_uri = field(query, "redirect_uri");
    let Ok(destination) = url::Url::parse(redirect_uri) else {
        return web_pages::invalid_request();
    };
    let Some(cancel) = client_redirect(
        redirect_uri,
        field(query, "state"),
        origin,
        &[("error", "access_denied")],
    ) else {
        return web_pages::invalid_request();
    };
    let rp = url::Url::parse(origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let passkey_available = web
        .authority
        .passkeys(&rp)
        .is_ok_and(|keys| !keys.is_empty());
    web_pages::consent(&web_pages::Consent {
        client,
        redirect_host: destination.authority(),
        scopes: &requested_scopes(query),
        cancel: &cancel,
        signed_in,
        passkey_available,
    })
}

fn decide(
    web: &Web,
    request: &axum::http::request::Parts,
    body: &[u8],
    origin: &str,
    query: &Query,
    client: &Client,
) -> Response {
    let invalid = || respond(400, &json!({"error": "invalid_request"}));
    let form = request
        .headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| {
            value
                .as_bytes()
                .starts_with(b"application/x-www-form-urlencoded")
        });
    let input: Value = if form {
        let posted = fields(&String::from_utf8_lossy(body));
        json!({
            "decision": field(&posted, "decision"),
            "level": field(&posted, "level"),
            "scopes": posted.get("scopes").cloned().unwrap_or_default(),
        })
    } else {
        let Ok(value) = serde_json::from_slice(body) else {
            return invalid();
        };
        value
    };
    let decision = input["decision"].as_str().unwrap_or_default();
    if !["approve", "reject"].contains(&decision) {
        return invalid();
    }
    let level = input["level"]
        .as_str()
        .filter(|level| !level.is_empty())
        .unwrap_or("approve");
    let grants = if decision == "approve" {
        let Ok(grants) = serde_json::from_value::<Vec<Grant>>(input["scopes"].clone()) else {
            return respond(400, &json!({"error": "invalid_scope"}));
        };
        match oauth::agent_grants(level, &grants) {
            Ok(grants) if ab_trust::subset(&grants, &requested_scopes(query)) => grants,
            _ => return respond(400, &json!({"error": "invalid_scope"})),
        }
    } else {
        Vec::new()
    };
    let Ok(destination) = crate::web_connector::decision(
        &web.authority,
        client,
        code_for(client, query, origin),
        field(query, "state"),
        origin,
        decision == "approve",
        level,
        &grants,
    ) else {
        return reject(503, "unavailable");
    };
    if form {
        web_pages::connector_redirect(&destination)
    } else {
        respond(200, &json!({"redirect": destination}))
    }
}

pub(crate) fn pending_request(web: &Web, origin: &str, ip: &str, query: &Query) -> Response {
    const INVALID: &str = "This connection request is invalid. Start again from your connector.";
    if !web.allow(&format!("connector-create:{ip}"), true, 50) {
        return reject(429, "Too many requests. Wait a minute and try again.");
    }
    if query.values().any(|values| values.len() != 1) || request_error(query, origin).is_some() {
        return reject(400, INVALID);
    }
    let Ok(client) = resolve_client(web, query) else {
        return reject(400, INVALID);
    };
    let code = code_for(&client, query, origin);
    web.connectors
        .begin(
            client,
            code,
            field(query, "state"),
            origin,
            requested_scopes(query),
        )
        .map_or_else(
            |_| reject(503, "Approval is unavailable. Try again in a few minutes."),
            |summary| respond(201, &summary),
        )
}

pub(crate) fn pending_poll(web: &Web, path: &str, origin: &str, ip: &str) -> Response {
    if !web.allow(&format!("connector-poll:{ip}"), false, 10) {
        return reject(429, "Wait a moment and try again.");
    }
    let id = url::form_urlencoded::parse(path.trim_start_matches("/oauth/pending/").as_bytes())
        .next()
        .map_or_else(String::new, |(key, _)| key.into_owned());
    web.connectors.poll(&id, origin).map_or_else(
        |_| {
            reject(
                410,
                "This approval has expired or was already used. Start again from your connector.",
            )
        },
        |redirect| respond(200, &json!({"redirect": redirect})),
    )
}
