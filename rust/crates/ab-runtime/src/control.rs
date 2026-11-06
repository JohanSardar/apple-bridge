use crate::Error;
use ab_operations::GatewayStore;
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub destination: std::path::PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Request {
    Backup(Backup),
    Helper(HelperRequest),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperRequest {
    action: HelperAction,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HelperAction {
    RestartHelper,
    HelperFence,
}

pub fn bind(home: &Path) -> Result<tokio::net::UnixListener, Error> {
    let path = home.join("run/control.sock");
    if path.try_exists()? {
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
pub async fn serve(
    listener: tokio::net::UnixListener,
    state: GatewayStore,
    helper: ab_applepeer::Commands,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let path = listener
        .local_addr()?
        .as_pathname()
        .ok_or(Error::Invalid)?
        .to_owned();
    let uid = std::fs::metadata(&path)?.uid();
    let mut requests = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = requests.join_next(), if !requests.is_empty() => {},
            accepted = listener.accept(), if requests.len() < 4 => {
                let (mut stream, _) = accepted?;
                if stream.peer_cred()?.uid() != uid { continue; }
                let state = state.clone();
                let helper = helper.clone();
                requests.spawn(async move {
                    let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        let size = stream.read_u32().await?;
                        if size == 0 || size > 4096 { return Err(Error::Invalid); }
                        let mut bytes = vec![0; size as usize];
                        stream.read_exact(&mut bytes).await?;
                        Ok::<Request, Error>(serde_json::from_slice(&bytes)?)
                    }).await;
                    let reply = match request {
                        Ok(Ok(Request::Backup(request))) => {
                            if tokio::task::spawn_blocking(move || state.with(|state| state.backup(request.destination))).await.is_ok_and(|result| result.is_ok()) {
                                "ok\n".to_owned()
                            } else { "failed\n".to_owned() }
                        }
                        Ok(Ok(Request::Helper(request))) => match request.action {
                            HelperAction::RestartHelper => helper.restart().await.map_or_else(|_| "failed\n".to_owned(), |fence| format!("{fence}\n")),
                            HelperAction::HelperFence => format!("{}\n", helper.current_fence()),
                        },
                        _ => "failed\n".to_owned(),
                    };
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        }
    }
    while requests.join_next().await.is_some() {}
    std::fs::remove_file(path)?;
    Ok(())
}
pub fn backup(home: &Path, destination: &Path) -> Result<(), Error> {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(home.join("run/control.sock"))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;
    let request = serde_json::to_vec(&Backup {
        destination: destination.to_owned(),
    })?;
    if request.len() > 4096 {
        return Err(Error::Invalid);
    }
    stream.write_all(
        &u32::try_from(request.len())
            .map_err(|_| Error::Invalid)?
            .to_be_bytes(),
    )?;
    stream.write_all(&request)?;
    let mut reply = String::new();
    stream.take(16).read_to_string(&mut reply)?;
    if reply != "ok\n" {
        return Err(Error::Invalid);
    }
    Ok(())
}

async fn helper_request(home: &Path, action: HelperAction) -> Result<u64, Error> {
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let mut stream = tokio::net::UnixStream::connect(home.join("run/control.sock")).await?;
        let request = serde_json::to_vec(&HelperRequest { action })?;
        stream
            .write_u32(u32::try_from(request.len()).map_err(|_| Error::Invalid)?)
            .await?;
        stream.write_all(&request).await?;
        let mut reply = String::new();
        stream.take(32).read_to_string(&mut reply).await?;
        reply.trim().parse().map_err(|_| Error::Invalid)
    })
    .await
    .map_err(|_| Error::Invalid)?
}

pub async fn restart_helper(home: &Path) -> Result<u64, Error> {
    helper_request(home, HelperAction::RestartHelper).await
}

pub async fn helper_fence(home: &Path) -> Result<u64, Error> {
    helper_request(home, HelperAction::HelperFence).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn helper_control_uses_the_gateway_and_preserves_backups()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        std::fs::create_dir_all(home.path().join("run"))?;
        let state = ab_state::State::<ab_state::Gateway>::open(home.path())?;
        let store = GatewayStore(std::sync::Arc::new(std::sync::Mutex::new(state)));
        let listener = bind(home.path())?;
        let (stop, receiver) = watch::channel(false);
        let serving = tokio::spawn(serve(
            listener,
            store,
            ab_applepeer::Commands::default(),
            receiver,
        ));
        assert_eq!(helper_fence(home.path()).await?, 0);
        assert!(restart_helper(home.path()).await.is_err());
        let path = home.path().to_owned();
        let destination = home.path().join("backup");
        tokio::task::spawn_blocking(move || backup(&path, &destination)).await??;
        assert_eq!(helper_fence(home.path()).await?, 0);
        stop.send(true)?;
        serving.await??;
        Ok(())
    }
}
