use crate::{http::Api, store::Device};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
struct Row {
    id: i64,
    name_value: String,
}
async fn fingerprints(
    client: &reqwest::Client,
    host: &str,
    origin: &str,
) -> crate::Result<Vec<String>> {
    let response = client
        .get(origin)
        .query(&[("q", host), ("output", "json")])
        .send()
        .await
        .map_err(|_| crate::Error::Denied)?;
    if response.status() != 200 {
        return Err(crate::Error::Denied);
    }
    let bytes = crate::client::limited(response, 2 << 20).await?;
    if bytes.len() > 2 << 20 {
        return Err(crate::Error::Denied);
    }
    let rows: Vec<Row> = serde_json::from_slice(&bytes)?;
    let mut result = Vec::new();
    for row in rows {
        if !row
            .name_value
            .split('\n')
            .any(|n| n.eq_ignore_ascii_case(host))
        {
            continue;
        }
        let response = client
            .get(origin)
            .query(&[("d", row.id)])
            .send()
            .await
            .map_err(|_| crate::Error::Denied)?;
        if response.status() != 200 {
            return Err(crate::Error::Denied);
        }
        let data = crate::client::limited(response, 65536).await?;
        if data.len() > 65536 {
            return Err(crate::Error::Denied);
        }
        let data = if let Ok(text) = std::str::from_utf8(&data) {
            if let Some((_, rest)) = text.split_once("-----BEGIN CERTIFICATE-----") {
                let (body, _) = rest
                    .split_once("-----END CERTIFICATE-----")
                    .ok_or(crate::Error::Denied)?;
                STANDARD
                    .decode(body.split_whitespace().collect::<String>())
                    .map_err(|_| crate::Error::Denied)?
            } else {
                data
            }
        } else {
            data
        };
        result.push(format!("{:x}", Sha256::digest(data)));
    }
    Ok(result)
}
async fn check(api: &Api, client: &reqwest::Client, mut device: Device) {
    let mut hosts = vec![device.hostname.clone()];
    if !device.domain.is_empty() && device.domain != device.hostname {
        hosts.push(device.domain.clone());
    }
    let mut warnings = Vec::new();
    for host in hosts {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            fingerprints(client, &host, &api.ct_origin),
        )
        .await;
        if let Some(current) = api
            .server
            .store
            .lock()
            .await
            .devices
            .get(&device.hostname)
            .cloned()
        {
            device = current;
        }
        let warning = match result {
            Ok(Ok(fps)) if fps.iter().any(|f| !device.certificates.contains(f)) => {
                "Unexpected certificate found in Certificate Transparency"
            }
            Ok(Ok(_)) => "",
            _ => "Certificate Transparency check unavailable",
        };
        if !warning.is_empty() {
            warnings.push(format!("{host}: {warning}"));
        }
    }
    api.warnings
        .lock()
        .await
        .insert(device.hostname, warnings.join("; "));
}
pub async fn monitor(api: Arc<Api>, stop: CancellationToken) {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    loop {
        tokio::select! { () = stop.cancelled() => return, () = monitor_once(&api,&client) => {} }
        tokio::select! { () = stop.cancelled() => return, () = tokio::time::sleep(Duration::from_secs(3600)) => {} }
    }
}
pub async fn monitor_once(api: &Api, client: &reqwest::Client) {
    let devices: Vec<_> = api
        .server
        .store
        .lock()
        .await
        .devices
        .values()
        .filter(|d| !d.revoked)
        .cloned()
        .collect();
    for d in devices {
        check(api, client, d).await;
    }
}
