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
                requests.spawn(async move {
                    let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        let size = stream.read_u32().await?;
                        if size == 0 || size > 4096 { return Err(Error::Invalid); }
                        let mut bytes = vec![0; size as usize];
                        stream.read_exact(&mut bytes).await?;
                        Ok::<Backup, Error>(serde_json::from_slice(&bytes)?)
                    }).await;
                    let success = if let Ok(Ok(request)) = request {
                        tokio::task::spawn_blocking(move || state.with(|state| state.backup(request.destination))).await.is_ok_and(|result| result.is_ok())
                    } else { false };
                    let _ = stream.write_all(if success { b"ok\n" } else { b"failed\n" }).await;
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
