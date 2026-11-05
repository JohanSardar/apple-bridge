use ab_applepeer::{Commands, Server};
use ab_operations::{Code, Error, peer::ApplePeer};
use ab_wire::messages::IpcFrame;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
include!("lifecycle.inc");
#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let commands = Commands::default();
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let state = Arc::new(ab_operations::GatewayStore::open(
        std::path::Path::new(args.get(1).ok_or("socket")?)
            .parent()
            .ok_or("parent")?
            .join("gateway-state"),
    )?);
    let server = Arc::new(Server {
        socket: args.get(1).ok_or("socket")?.into(),
        secret: [7; 32],
        backend: state.clone(),
        commands: commands.clone(),
    });
    let task = tokio::spawn(server.serve(receiver));
    let mut local = ab_operations::local::LocalGateway::new(
        state.clone(),
        Arc::new(Identity),
        Arc::new(Codec),
        Arc::new(commands.clone()),
        Arc::new(Actions),
    );
    #[cfg(feature = "lab-harness")]
    {
        local.clock = Arc::new(ab_operations::clock::LabClock(
            std::path::Path::new(args.get(1).ok_or("socket")?)
                .parent()
                .ok_or("parent")?
                .to_path_buf(),
        ));
    }
    local.query_store = Some(Arc::new(ab_operations::LocalQueryStore::open(
        (*state).clone(),
        std::path::Path::new(args.get(1).ok_or("socket")?)
            .parent()
            .ok_or("parent")?
            .join("gateway-blobs"),
        std::env::var("AB_PROBE_BLOB_CAP")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()?
            .unwrap_or(128 << 20),
    )?));
    let audit = Arc::new(Audit::default());
    local.query_audit = Some(audit.clone());
    let mut gateway = Arc::new(local);
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut saved_blob: Option<(ab_operations::peer::PeerSession, String, i64)> = None;
    while let Some(line) = lines.next_line().await? {
        let frame: IpcFrame = serde_json::from_str(&line)?;
        let result = if frame.t == "DisableUploads" {
            Arc::get_mut(&mut gateway)
                .ok_or("active gateway")?
                .query_store = None;
            serde_json::json!({"disabled":true})
        } else if frame.t == "StartBlob" {
            saved_blob = Some((
                commands.session().ok_or("session")?,
                frame.handle,
                frame.length,
            ));
            serde_json::json!({"started":true})
        } else if frame.t == "FinishBlob" {
            let (session, handle, size) = saved_blob.take().ok_or("saved blob")?;
            match commands.read_blob_from(session, &handle, 0, size).await {
                Ok(_) => serde_json::json!({"read":true}),
                Err(error) => serde_json::json!({"error":{"code":error.code}}),
            }
        } else if frame.t == "QueryAudits" {
            serde_json::to_value(&*audit.0.lock().map_err(|_| "audit lock")?)?
        } else if matches!(
            frame.t.as_str(),
            "Submit" | "Status" | "Decide" | "Recover" | "LocalQuery"
        ) {
            let p = principal(if frame.principal.is_empty() {
                "device:probe"
            } else {
                &frame.principal
            });
            let result = match frame.t.as_str() {
                "LocalQuery" => {
                    gateway
                        .query(
                            &p,
                            serde_json::from_str(frame.args.as_ref().ok_or("query")?.get())?,
                        )
                        .await
                }
                "Submit" => {
                    gateway
                        .submit(
                            &p,
                            serde_json::from_str(frame.args.as_ref().ok_or("command")?.get())?,
                        )
                        .await
                }
                "Status" => {
                    gateway
                        .status(
                            &p,
                            &ab_operations::CommandQuery {
                                id: frame.command_id,
                                resolve: frame.code == "resolve",
                                limit: 0,
                            },
                        )
                        .await
                }
                "Decide" => {
                    gateway
                        .decide(
                            &p,
                            &frame.command_id,
                            frame.code == "approve",
                            &ab_operations::approval_review::OwnerReview {
                                binding: "",
                                accepted_unpreviewable: false,
                            },
                        )
                        .await
                }
                _ => gateway.recover().and_then(|()| {
                    serde_json::value::to_raw_value(&true).map_err(|_| Code::Corrupt.into())
                }),
            };
            match result {
                Ok(raw) => serde_json::from_str(raw.get())?,
                Err(error) => serde_json::json!({"error":error}),
            }
        } else if frame.t == "Available" {
            serde_json::json!({"available": commands.available()})
        } else if frame.t == "ReadBlob" {
            let result = async {
                let mut data = Vec::new();
                while i64::try_from(data.len()).map_err(|_| Code::Corrupt)? < frame.length {
                    let offset = i64::try_from(data.len()).map_err(|_| Code::Corrupt)?;
                    let (bytes, done) = commands
                        .read_blob(
                            &frame.handle,
                            offset,
                            (frame.length - offset).min(128 << 10),
                        )
                        .await?;
                    if bytes.is_empty()
                        || offset + i64::try_from(bytes.len()).map_err(|_| Code::Corrupt)?
                            > frame.length
                    {
                        return Err(Error::from(Code::Corrupt));
                    }
                    data.extend(bytes);
                    if done && i64::try_from(data.len()).map_err(|_| Code::Corrupt)? != frame.length
                    {
                        return Err(Error::from(Code::Corrupt));
                    }
                }
                Ok(data)
            }
            .await;
            match result {
                Ok(bytes) => {
                    use base64::Engine;
                    serde_json::json!({"bytes":base64::engine::general_purpose::STANDARD.encode(bytes),"eof":true})
                }
                Err(error) => serde_json::json!({"error":error}),
            }
        } else if frame.t == "StaleDispatch" {
            #[cfg(feature = "lab-harness")]
            {
                let mut frame = frame;
                frame.t = "Dispatch".into();
                frame.owner_fence = 0;
                serde_json::to_value(commands.exchange_unfenced(frame).await?)?
            }
            #[cfg(not(feature = "lab-harness"))]
            {
                return Err("lab-harness feature required".into());
            }
        } else {
            match tokio::time::timeout(std::time::Duration::from_secs(3), commands.exchange(frame))
                .await
            {
                Ok(Ok(answer)) => serde_json::to_value(answer)?,
                Ok(Err(error)) => serde_json::json!({"error": error}),
                Err(_) => serde_json::json!({"error": {"code": Code::Deadline}}),
            }
        };
        let mut bytes = serde_json::to_vec(&result)?;
        bytes.push(b'\n');
        tokio::io::stdout().write_all(&bytes).await?;
    }
    stop.send(true)?;
    task.await??;
    Ok(())
}
