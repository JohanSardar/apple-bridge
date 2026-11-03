use crate::{Error, Result, state};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

fn clean(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other),
        }
    }
    clean
}

pub fn control_path(home: &Path, id: &str) -> Result<PathBuf> {
    let absolute = if home.is_absolute() {
        home.to_path_buf()
    } else {
        std::env::current_dir()?.join(home)
    };
    let canonical = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    let uid = std::fs::metadata(home.join("remote"))?.uid();
    let mut digest = Sha256::new();
    digest.update(format!("{}/{id}", clean(&canonical).display()));
    let digest = digest.finalize();
    Ok(PathBuf::from(format!("/tmp/ab-remote-{uid}")).join(format!("{}.sock", hex(&digest[..16]))))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn valid_link(link: &str) -> bool {
    link.strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|host| !host.is_empty() && !host.contains('@'))
}

pub fn open_link(link: &str) -> bool {
    use std::process::{Command, Stdio};
    if !valid_link(link) {
        return false;
    }
    let launcher = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let Ok(mut child) = Command::new(launcher)
        .arg(link)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub async fn send_link(home: &Path, id: &str, link: &str) -> Result<()> {
    let path = control_path(home, id)?;
    let mut stream = tokio::time::timeout(Duration::from_secs(1), UnixStream::connect(path))
        .await
        .map_err(|_| Error::Contract("approval IPC timeout"))??;
    let mut line = serde_json::to_vec(link)?;
    line.push(b'\n');
    tokio::time::timeout(Duration::from_secs(1), stream.write_all(&line))
        .await
        .map_err(|_| Error::Contract("approval IPC timeout"))??;
    Ok(())
}

fn listener(home: &Path, id: &str) -> Result<(UnixListener, PathBuf)> {
    let remote = home.join("remote");
    std::fs::create_dir_all(&remote)?;
    std::fs::set_permissions(&remote, std::fs::Permissions::from_mode(0o700))?;
    let path = control_path(home, id)?;
    let directory = path
        .parent()
        .ok_or(Error::Contract("missing approval IPC directory"))?;
    std::fs::create_dir_all(directory)?;
    let uid = std::fs::metadata(&remote)?.uid();
    let info = std::fs::symlink_metadata(directory)?;
    if info.file_type().is_symlink() || info.uid() != uid {
        return Err(Error::Contract("approval IPC directory must be private"));
    }
    if info.permissions().mode() & 0o777 != 0o700 {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, path))
}

pub type Opener = std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub async fn command(
    home: &Path,
    action: &str,
    via: &str,
    out: &mut (dyn Write + Send),
    opener: Opener,
    recheck: Duration,
) -> Result<()> {
    if action == "status" {
        writeln!(out, "{}", serde_json::to_string(&state::status(home)?)?)?;
        return Ok(());
    }
    if action != "on" && action != "off" {
        return Err(Error::Contract("action must be on, off or status"));
    }
    let id = state::token();
    let (control, socket) = listener(home, &id)?;
    let (links, mut received) = tokio::sync::mpsc::channel::<String>(16);
    let accepting = tokio::spawn(async move {
        while let Ok((stream, _)) = control.accept().await {
            let links = links.clone();
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(link) = serde_json::from_str::<String>(&line)
                        && links.send(link).await.is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    let result = wait(home, action, via, &id, out, &opener, recheck, &mut received).await;
    accepting.abort();
    let _ = std::fs::remove_file(socket);
    result
}

#[allow(clippy::too_many_arguments)]
async fn wait(
    home: &Path,
    action: &str,
    via: &str,
    id: &str,
    out: &mut (dyn Write + Send),
    opener: &Opener,
    recheck: Duration,
    links: &mut tokio::sync::mpsc::Receiver<String>,
) -> Result<()> {
    state::select(home, action, via, id)?;
    let mut ticker = tokio::time::interval(recheck);
    let mut notice = String::new();
    let mut opened = BTreeSet::new();
    loop {
        if let Ok(status) = state::read::<state::Status>(home, "status")
            && status.id == id
        {
            if !status.message.is_empty() && status.message != notice {
                writeln!(out, "{}", status.message)?;
                notice.clone_from(&status.message);
            }
            if status.state == "on" || status.state == "off" {
                if status.url.is_empty() {
                    writeln!(out, "{}", status.state)?;
                } else {
                    writeln!(out, "{}", status.url)?;
                    writeln!(out, "{}/mcp", status.url)?;
                }
                return Ok(());
            }
            if status.state == "error" {
                return Err(Error::Remote(format!("remote: {}", status.last_error)));
            }
        }
        if state::read::<state::Config>(home, "config").is_ok_and(|current| current.id != id) {
            return Err(Error::Remote("remote request superseded".into()));
        }
        tokio::select! {
            Some(link) = links.recv() => {
                if opened.insert(link.clone()) {
                    writeln!(out, "{link}")?;
                    let launch = link.clone();
                    let call = opener.clone();
                    let launched = tokio::task::spawn_blocking(move || call(&launch))
                        .await
                        .unwrap_or(false);
                    if !launched {
                        writeln!(out, "Open the link above in your browser")?;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                state::cancel(home, id, via)?;
                return Err(Error::Remote("interrupted".into()));
            }
            _ = ticker.tick() => {}
        }
    }
}
