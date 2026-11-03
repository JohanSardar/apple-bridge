use crate::{Client, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::SocketAddr, time::Instant};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Input {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "IP")]
    pub ip: String,
    pub remote: String,
    pub local: String,
    #[serde(default)]
    pub off: bool,
}
fn split(url: &str) -> Result<(String, u16)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or(crate::Error::Invalid("url"))?
        .trim_end_matches('/');
    Ok(match rest.rsplit_once(':') {
        Some((host, port)) => (
            host.to_owned(),
            port.parse().map_err(|_| crate::Error::Invalid("port"))?,
        ),
        None => (rest.to_owned(), 443),
    })
}
fn reply(
    client: &Client,
    method: &str,
    path: &str,
    token: &str,
    origin: &str,
) -> Option<crate::Reply> {
    let mut headers = vec![("Content-Type", "application/json")];
    let bearer = format!("Bearer {token}");
    if !token.is_empty() {
        headers.push(("Authorization", &bearer));
    }
    if !origin.is_empty() {
        headers.push(("Origin", origin));
        headers.push(("X-Forwarded-Host", "spoof.invalid"));
        headers.push(("X-Forwarded-Proto", "http"));
    }
    client
        .request(method, path, &headers, br#"{"limit":1}"#)
        .ok()
}
fn status(client: &Client, method: &str, path: &str, token: &str, origin: &str) -> u16 {
    reply(client, method, path, token, origin).map_or(0, |r| r.status)
}
fn mcp(client: &Client, token: &str, session: &str, body: &Value) -> Option<(crate::Reply, Value)> {
    let bearer = format!("Bearer {token}");
    let mut headers = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("Authorization", bearer.as_str()),
    ];
    if !session.is_empty() {
        headers.push(("Mcp-Session-Id", session));
    }
    let reply = client
        .request("POST", "/mcp", &headers, body.to_string().as_bytes())
        .ok()?;
    let text = String::from_utf8_lossy(&reply.body).into_owned();
    let payload = if reply
        .header("content-type")
        .is_some_and(|t| t.starts_with("text/event-stream"))
    {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .find_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
    } else {
        serde_json::from_str(&text).ok()
    }
    .unwrap_or(Value::Null);
    Some((reply, payload))
}
pub fn checks(input: &Input, roots: rustls::RootCertStore) -> Result<Value> {
    let (host, port) = split(&input.url)?;
    let target = SocketAddr::new(
        input.ip.parse().map_err(|_| crate::Error::Invalid("ip"))?,
        port,
    );
    let client = Client::new(&host, port, target, roots)?;
    let mut results: BTreeMap<String, bool> = BTreeMap::new();
    if input.off {
        results.insert(
            "off_public_stopped".into(),
            status(&client, "GET", "/v1/health", "", "") != 200,
        );
        return Ok(json!({"checks": results}));
    }
    let page = reply(&client, "GET", "/", "", "");
    results.insert(
        "html_valid_certificate".into(),
        page.is_some_and(|r| {
            r.status == 200 && String::from_utf8_lossy(&r.body).contains("id=\"root\"")
        }),
    );
    results.insert(
        "public_health".into(),
        status(&client, "GET", "/v1/health", "", "") == 200,
    );
    let query = "/v1/op/records.query";
    results.insert(
        "remote_token_rest".into(),
        status(&client, "POST", query, &input.remote, "") == 200,
    );
    results.insert(
        "local_token_rejected".into(),
        status(&client, "POST", query, &input.local, "") == 401,
    );
    results.insert(
        "origin_rejected".into(),
        status(&client, "POST", query, &input.remote, &input.url) == 401,
    );
    results.insert(
        "feed_requires_replica_handshake".into(),
        [400, 401, 426].contains(&status(&client, "GET", "/v1/feed", &input.remote, "")),
    );
    let options = reply(&client, "POST", "/auth/login/options", "", &input.url);
    let rp = options
        .as_ref()
        .and_then(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .and_then(|v| v["publicKey"]["rpId"].as_str().map(str::to_owned));
    let options_ok = options.as_ref().is_some_and(|r| r.status == 200);
    results.insert("forwarded_headers_ignored".into(), options_ok);
    results.insert(
        "public_passkey_origin_rpid".into(),
        options_ok && rp.as_deref() == Some(host.as_str()),
    );
    mcp_checks(&client, &input.remote, &mut results);
    let mut values = Vec::new();
    for index in 0..20 {
        let started = Instant::now();
        let ok = status(&client, "POST", query, &input.remote, "") == 200;
        results.insert(format!("read_{index:02}"), ok);
        values.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    values.sort_by(f64::total_cmp);
    Ok(json!({
        "checks": results,
        "latency": {"samples": values.len(), "p50_ms": values[9], "p95_ms": values[18], "max_ms": values[19]},
    }))
}
fn mcp_checks(client: &Client, token: &str, results: &mut BTreeMap<String, bool>) {
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"bridge-funnel-lab","version":"1"}}});
    let started = mcp(client, token, "", &init);
    let initialized = started
        .as_ref()
        .is_some_and(|(r, v)| r.status == 200 && v["result"].is_object());
    results.insert("mcp_initialize".into(), initialized);
    let Some((first, _)) = started.filter(|_| initialized) else {
        return;
    };
    let session = first
        .header("mcp-session-id")
        .unwrap_or_default()
        .to_owned();
    let _ = mcp(
        client,
        token,
        &session,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    let tools = mcp(
        client,
        token,
        &session,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    );
    results.insert(
        "mcp_tools_list".into(),
        tools.as_ref().is_some_and(|(_, v)| {
            v["result"]["tools"]
                .as_array()
                .is_some_and(|t| !t.is_empty())
        }),
    );
    let call = mcp(
        client,
        token,
        &session,
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"records_query","arguments":{"limit":1}}}),
    );
    results.insert(
        "mcp_read_tool".into(),
        call.as_ref()
            .is_some_and(|(_, v)| v["result"].is_object() && v["result"]["isError"] != true),
    );
    results.insert(
        "mcp_sse_response".into(),
        [tools, call].iter().flatten().any(|(r, _)| {
            r.header("content-type")
                .is_some_and(|t| t.starts_with("text/event-stream"))
        }),
    );
}
