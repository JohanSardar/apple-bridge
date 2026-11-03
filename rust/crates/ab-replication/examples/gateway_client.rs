use ab_replication::Client;
use ab_trust::Authority;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let code = std::env::args().nth(2).ok_or("code")?;
    let authority = Arc::new(Authority::open(&home)?);
    if code != "-" {
        let decoded = ab_trust::PairCode::decode(&code)?;
        if decoded.dial.is_empty() {
            authority.join(&code).await?;
        } else {
            authority.join_listen(&code, &decoded.dial).await?;
        }
    }
    let state = Arc::new(Mutex::new(ab_state::State::<ab_state::Replica>::open(
        &home,
    )?));
    let client = Client::new(
        state.clone(),
        authority,
        Some(Arc::new(Mutex::new(ab_blobs::Store::open(&home, 0)?))),
    );
    let (_stop, rx) = watch::channel(false);
    let mut status = client.status();
    let monitor = async {
        loop {
            tokio::select! { r = status.changed() => r?, () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {} }
            let state = state.lock().await;
            let records = state.read(
                &ab_trust::Principal {
                    grants: vec![ab_trust::Grant::parse("reminders:read")?],
                    ..ab_trust::Principal::default()
                }
                .visibility()
                .to_map(),
                &ab_state::ReadQuery::default(),
            )?;
            ab_trust::atomic_write(
                &std::path::Path::new(&home).join("state.json"),
                &serde_json::to_vec(
                    &serde_json::json!({"cursor":records.cursor,"digest":ab_state::digest(&records.records)}),
                )?,
            )?;
            ab_trust::atomic_write(
                &std::path::Path::new(&home).join("status.json"),
                &serde_json::to_vec(&*status.borrow())?,
            )?;
        }
        #[allow(unreachable_code)]
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let upload = async {
        if code == "-" {
            std::future::pending::<()>().await;
        }
        let transport = client.gateway_transport();
        while !transport.connected() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let principal = ab_trust::Principal {
            id: "token:fixture".into(),
            kind: "token".into(),
            grants: vec![ab_trust::Grant::parse("messages:write")?],
            ..Default::default()
        };
        let id = format!("upload_01ARZ3NDEKTSV4RRFFQ69G5FAV{}", std::process::id());
        let bytes = b"Rust synthetic upload";
        let digest = ab_trust::fingerprint(bytes);
        for action in ["begin", "prepare", "commit"] {
            let request = ab_blobs::uploads::UploadRequest {
                action: action.into(),
                upload_id: id.clone(),
                chunk_id: if action == "prepare" {
                    "request_01ARZ3NDEKTSV4RRFFQ69G5FAV".into()
                } else {
                    String::new()
                },
                offset: 0,
                size: i64::try_from(bytes.len())?,
                digest: digest.clone(),
            };
            let raw = ab_trust::wire::gojson::legacy_json(
                &request,
                ab_trust::wire::gojson::EscapeMode::Html,
            )?;
            let header = client.authority.sign_delegation(
                &principal,
                &id,
                &ab_trust::fingerprint(&raw),
                "",
            )?;
            let response = transport.upload_control(header, raw).await?;
            if let Some(error) = response.error {
                return Err(format!("Rust upload {action}: {error:?}").into());
            }
            if action == "prepare" {
                let response = transport
                    .upload_chunk(
                        request.chunk_id,
                        hex::decode(&digest)?.try_into().map_err(|_| "digest")?,
                        0,
                        bytes.to_vec(),
                    )
                    .await?;
                if response.error.is_some() {
                    return Err("Rust upload chunk".into());
                }
            }
        }
        ab_trust::atomic_write(&std::path::Path::new(&home).join("upload.json"), b"true")?;
        std::future::pending::<()>().await;
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let feed = async {
        let enrollment = client.authority.enrollment()?.ok_or("enrollment")?;
        if enrollment.dial.is_empty() {
            client.follow(rx).await?;
        } else {
            client.listen(&enrollment.dial, rx).await?;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    tokio::select! { result = feed => result?, result = monitor => result?, result = upload => result? }
    Ok(())
}
