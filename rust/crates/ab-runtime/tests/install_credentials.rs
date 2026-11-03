use std::{path::Path, process::Command};

fn bridge(home: &Path, config: &Path, store: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bridge"));
    command
        .args(["install-credentials", "--home"])
        .arg(home)
        .env("XDG_CONFIG_HOME", config)
        .env("PATH", "/nonexistent")
        .env_remove("APPLE_BRIDGE_SECRET_STORE");
    if let Some(store) = store {
        command.env("APPLE_BRIDGE_SECRET_STORE", store);
    }
    command.output().unwrap()
}

#[test]
fn install_credentials_creates_the_identity_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let first = bridge(&home, &root.path().join("config"), Some("file"));
    assert!(first.status.success(), "{first:?}");
    assert!(first.stdout.is_empty());
    let certificate = std::fs::read(home.join("identity.pem")).unwrap();
    assert_eq!(std::fs::read_dir(home.join("secrets")).unwrap().count(), 1);
    assert!(!home.join("identity-key.pem").exists());
    let second = bridge(&home, &root.path().join("config"), Some("file"));
    assert!(second.status.success(), "{second:?}");
    assert_eq!(
        std::fs::read(home.join("identity.pem")).unwrap(),
        certificate
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_default_keeps_the_identity_across_binary_swaps() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let config = root.path().join("config");
    assert!(bridge(&home, &config, None).status.success());
    let certificate = std::fs::read(home.join("identity.pem")).unwrap();
    assert!(!home.join("secrets").exists());
    let stores = std::fs::read_dir(config.join("apple-bridge/secrets"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(stores.len(), 1);
    assert_eq!(std::fs::read_dir(stores[0].path()).unwrap().count(), 1);
    assert!(bridge(&home, &config, None).status.success());
    assert_eq!(
        std::fs::read(home.join("identity.pem")).unwrap(),
        certificate
    );
    assert_eq!(
        std::fs::read_dir(config.join("apple-bridge/secrets"))
            .unwrap()
            .count(),
        1
    );
}
