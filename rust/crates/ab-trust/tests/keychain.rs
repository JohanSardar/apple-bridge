#![cfg(target_os = "macos")]

use ab_trust::{CredentialStore, Error, KeychainStore};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

type Check = Result<(), Box<dyn std::error::Error>>;

const GO_SCRIPT: &str = include_str!("../src/go_keychain.py");

fn search_list() -> Result<String, std::io::Error> {
    let output = Command::new("security")
        .args(["list-keychains", "-d", "user"])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new("security")
            .arg("delete-keychain")
            .arg(&self.0)
            .output();
    }
}

fn go_store(
    operation: &str,
    keychain: &Path,
    service: &str,
    key: &str,
    value: &[u8],
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let request = serde_json::json!({
        "op": operation,
        "path": keychain,
        "service": service,
        "key": key,
        "value": data_encoding::HEXLOWER.encode(value),
    });
    let mut child = Command::new("python3")
        .args(["-c", GO_SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(request.to_string().as_bytes())?;
    let output = child.wait_with_output()?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn managed_temporary_keychain_round_trips_and_reads_go_items() -> Check {
    let before = search_list()?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("Apple Bridge.keychain-db");
    let _cleanup = Cleanup(path.clone());
    let service = "apple-bridge:keychain-test";
    let store = KeychainStore::managed(service.into(), path.clone());

    assert!(matches!(
        store.get("identity-key"),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound
    ));
    assert_eq!(search_list()?, before);
    assert!(path.exists());
    assert_eq!(
        go_store("put", &path, service, "pre", b"seed")?["status"],
        0
    );
    assert_eq!(store.get("pre")?, b"seed");
    assert_eq!(store.get("pre")?, b"seed");
    store.put("pre", b"seed2")?;
    assert_eq!(store.get("pre")?, b"seed2");
    let still_go = go_store("get", &path, service, "pre", b"")?;
    assert_eq!(still_go["status"], 0);
    assert_eq!(
        data_encoding::HEXLOWER.decode(still_go["value"].as_str().ok_or("value")?.as_bytes())?,
        b"seed2"
    );
    store.put("identity-key", b"first value")?;
    assert_eq!(store.get("identity-key")?, b"first value");
    store.put("identity-key", &[0, 1, 2, 255])?;
    assert_eq!(store.get("identity-key")?, [0, 1, 2, 255]);

    let go_status = go_store("put", &path, service, "go-key", b"written by go")?;
    assert_eq!(go_status["status"], 0);
    assert_eq!(store.get("go-key")?, b"written by go");

    store.put("rust-key", b"written by rust")?;
    assert_eq!(store.get("rust-key")?, b"written by rust");
    store.delete("rust-key")?;

    store.delete("go-key")?;
    store.delete("go-key")?;
    assert!(matches!(
        store.get("go-key"),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound
    ));
    assert_eq!(
        go_store("get", &path, service, "go-key", b"")?["status"],
        -25300
    );
    Ok(())
}
