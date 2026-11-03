use ab_applepeer::{Backend, Commands, Server};
use ab_operations::{Code, Cursor, Task, peer::ApplePeer};
use ab_wire::messages::{Capture, IpcFrame};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
#[derive(Debug, Default)]
struct Memory(AtomicU64);
impl Backend for Memory {
    fn next_fence(&self) -> Task<'_, u64> {
        Box::pin(async { Ok(self.0.fetch_add(1, Ordering::SeqCst) + 1) })
    }
    fn checkpoints(&self) -> Task<'_, BTreeMap<String, Box<serde_json::value::RawValue>>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
    fn commit(&self, _: Capture) -> Task<'_, (Cursor, bool)> {
        Box::pin(async {
            Ok((
                Cursor {
                    epoch: "probe".into(),
                    subscription_revision: 1,
                    sequence: self.0.fetch_add(1, Ordering::SeqCst) + 1,
                },
                false,
            ))
        })
    }
    fn source_state(&self, _: IpcFrame) -> Task<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
include!("lifecycle.inc");
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let commands = Commands::default();
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let server = Arc::new(Server {
        socket: args.get(1).ok_or("socket")?.into(),
        secret: [7; 32],
        backend: Arc::new(Memory::default()),
        commands: commands.clone(),
    });
    let task = tokio::spawn(server.serve(receiver));
    let gateway = Arc::new(ab_operations::local::LocalGateway::new(
        Arc::new(Ledger::default()),
        Arc::new(Identity),
        Arc::new(Codec),
        Arc::new(commands.clone()),
        Arc::new(Actions),
    ));
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let frame: IpcFrame = serde_json::from_str(&line)?;
        let result = if matches!(frame.t.as_str(), "Submit" | "Status" | "Decide" | "Recover") {
            let p = principal(if frame.principal.is_empty() {
                "device:probe"
            } else {
                &frame.principal
            });
            let result = match frame.t.as_str() {
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
                        .decide(&p, &frame.command_id, frame.code == "approve")
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
