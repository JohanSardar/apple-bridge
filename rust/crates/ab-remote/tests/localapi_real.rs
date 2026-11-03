use ab_remote::localapi::{LocalApi, certificate_expiry};
use serde_json::Value;

#[tokio::test]
#[ignore = "reads the real Tailscale app on this Mac; read-only GETs"]
async fn real_app_status_and_serve_config_are_readable() {
    let api = LocalApi::Discover;
    let resolved = api.resolve().await;
    assert!(
        matches!(resolved, LocalApi::Tcp { .. }),
        "no Tailscale app credentials: {resolved:?}"
    );
    let status: Value = api
        .request(reqwest::Method::GET, "status?peers=false", None, None)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["BackendState"], "Running");
    let host = status["Self"]["DNSName"]
        .as_str()
        .unwrap()
        .trim_end_matches('.')
        .to_owned();
    assert!(host.ends_with(".ts.net"));
    assert!(status["Self"]["CapMap"].get("https").is_some());
    assert!(status["Self"]["CapMap"].get("funnel").is_some());
    let response = api
        .request(reqwest::Method::GET, "serve-config", None, None)
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let etag = response
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(!etag.is_empty());
    let config: Value = response.json().await.unwrap();
    println!(
        "serve-config sections: {:?}",
        config.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );
    let port = config["TCP"]
        .as_object()
        .and_then(|tcp| tcp.iter().find(|(_, v)| v["HTTPS"] == true))
        .and_then(|(port, _)| port.parse::<u16>().ok());
    if let Some(port) = port {
        let expiry = certificate_expiry(&host, port).await.unwrap();
        println!("public certificate expiry on port {port}: {expiry}");
        assert!(expiry.ends_with('Z') && !expiry.starts_with("0001"));
    }
}
