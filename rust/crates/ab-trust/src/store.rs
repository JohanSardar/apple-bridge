use crate::{CredentialStoreError, Error, fingerprint};
use rand::RngCore;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub trait CredentialStore: std::fmt::Debug + Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error>;
    fn get(&self, key: &str) -> Result<Vec<u8>, Error>;
    fn delete(&self, key: &str) -> Result<(), Error>;
}
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
}
impl FileStore {
    pub fn new(home: &Path) -> Result<Self, Error> {
        Self::at(home.join("secrets"))
    }
    pub fn at(dir: PathBuf) -> Result<Self, Error> {
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self { dir })
    }
}
impl CredentialStore for FileStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error> {
        atomic_write(&self.dir.join(fingerprint(key.as_bytes())), bytes)
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let path = self.dir.join(fingerprint(key.as_bytes()));
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 {
            return Err(Error::Unauthorized);
        }
        Ok(fs::read(path)?)
    }
    fn delete(&self, key: &str) -> Result<(), Error> {
        match fs::remove_file(self.dir.join(fingerprint(key.as_bytes()))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
#[derive(Debug)]
pub struct LibsecretStore {
    program: PathBuf,
    service: String,
}
impl LibsecretStore {
    #[must_use]
    pub fn new(program: PathBuf, service: String) -> Self {
        Self { program, service }
    }
    fn run(&self, input: &[u8], arguments: &[&str]) -> Result<Vec<u8>, Error> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(&self.program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::Unavailable)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input).map_err(|_| Error::Unavailable)?;
        }
        let mut stdout = child.stdout.take().ok_or(Error::Unavailable)?;
        let reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            std::io::Read::read_to_end(&mut stdout, &mut buffer).map(|_| buffer)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            match child.try_wait().map_err(|_| Error::Unavailable)? {
                Some(status) => break status,
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Unavailable);
                }
                None => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        };
        let output = reader
            .join()
            .map_err(|_| Error::Unavailable)?
            .map_err(|_| Error::Unavailable)?;
        match status.code() {
            Some(0) => Ok(output.trim_ascii().to_vec()),
            Some(1) => Err(Error::Io(std::io::ErrorKind::NotFound.into())),
            _ => Err(Error::Unavailable),
        }
    }
}
impl CredentialStore for LibsecretStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error> {
        self.run(
            hex::encode(bytes).as_bytes(),
            &[
                "store",
                "--label=Apple Bridge",
                "application",
                &self.service,
                "key",
                key,
            ],
        )
        .map(|_| ())
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let output = self.run(b"", &["lookup", "application", &self.service, "key", key])?;
        hex::decode(output).map_err(|_| Error::Unavailable)
    }
    fn delete(&self, key: &str) -> Result<(), Error> {
        match self.run(b"", &["clear", "application", &self.service, "key", key]) {
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map(|_| ()),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct StoreEnvironment {
    pub secret_store: Option<String>,
    pub xdg_config_home: Option<String>,
    pub home: Option<String>,
    pub secret_tool: Option<PathBuf>,
    pub bundle: Option<PathBuf>,
}
impl StoreEnvironment {
    #[must_use]
    pub fn current() -> Self {
        let path = std::env::var_os("PATH").unwrap_or_default();
        Self {
            secret_store: std::env::var("APPLE_BRIDGE_SECRET_STORE").ok(),
            xdg_config_home: std::env::var("XDG_CONFIG_HOME").ok(),
            home: std::env::var("HOME").ok(),
            bundle: std::env::current_exe().ok().and_then(|exe| {
                let bundle = exe.parent()?.parent()?.parent()?.to_owned();
                bundle
                    .join("Contents/Library/LaunchAgents/dev.applebridge.gateway.plist")
                    .exists()
                    .then_some(bundle)
            }),
            secret_tool: std::env::split_paths(&path)
                .map(|dir| dir.join("secret-tool"))
                .find(|candidate| {
                    fs::metadata(candidate)
                        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                }),
        }
    }
}
fn absolute(path: &Path) -> PathBuf {
    use std::path::Component;
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut clean = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                clean.pop();
            }
            Component::Normal(part) => clean.push(part),
            _ => {}
        }
    }
    clean
}
pub fn select(
    home: &Path,
    environment: &StoreEnvironment,
) -> Result<Box<dyn CredentialStore>, Error> {
    if environment.secret_store.as_deref() == Some("file") {
        return Ok(Box::new(FileStore::at(home.join("secrets"))?));
    }
    if cfg!(target_os = "macos") {
        if environment.bundle.is_some() {
            return Ok(Box::new(KeychainStore::managed(
                service_for(home),
                home.join("Apple Bridge.keychain-db"),
            )));
        }
        if environment.secret_store.as_deref() == Some("keychain") {
            return Ok(Box::new(KeychainStore::login(service_for(home))));
        }
        return Ok(Box::new(FileStore::at(home.join("secrets"))?));
    }
    let absolute = absolute(home);
    let digest = fingerprint(absolute.as_os_str().as_encoded_bytes());
    let service = format!("apple-bridge:{digest}");
    if let Some(program) = &environment.secret_tool {
        return Ok(Box::new(LibsecretStore::new(program.clone(), service)));
    }
    let root = match environment.xdg_config_home.as_deref() {
        Some(root) if !root.is_empty() => PathBuf::from(root),
        _ => PathBuf::from(environment.home.clone().unwrap_or_default()).join(".config"),
    };
    Ok(Box::new(FileStore::at(
        root.join("apple-bridge").join("secrets").join(digest),
    )?))
}
#[derive(Debug)]
pub struct KeychainStore {
    service: String,
    path: PathBuf,
    managed: bool,
}
const LOCKED_REMEDY: &str =
    "Unlock the login Keychain in an interactive macOS session, then retry bridge start.";
const UNAVAILABLE_REMEDY: &str =
    "Check access to the login Keychain in an interactive macOS session, then retry bridge start.";
fn keychain_error(locked: bool) -> Error {
    CredentialStoreError {
        code: if locked {
            "credential_store_locked"
        } else {
            "credential_store_unavailable"
        }
        .into(),
        store: "keychain".into(),
        remedy: if locked {
            LOCKED_REMEDY
        } else {
            UNAVAILABLE_REMEDY
        }
        .into(),
        retryable: true,
    }
    .into()
}
pub fn service_for(home: &Path) -> String {
    format!(
        "apple-bridge:{}",
        fingerprint(absolute(home).as_os_str().as_encoded_bytes())
    )
}
fn security(arguments: &[&str]) -> Result<Vec<u8>, Error> {
    let mut child = Command::new("security")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| keychain_error(false))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => {
                let mut output = Vec::new();
                if let Some(mut stdout) = child.stdout.take() {
                    stdout.read_to_end(&mut output)?;
                }
                return Ok(output.trim_ascii().to_vec());
            }
            Some(status) => return Err(keychain_error(status.code() == Some(36))),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(keychain_error(true));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}
impl KeychainStore {
    pub fn login(service: String) -> Self {
        Self {
            service,
            path: PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/Keychains/login.keychain-db"),
            managed: false,
        }
    }
    pub fn managed(service: String, path: PathBuf) -> Self {
        Self {
            service,
            path,
            managed: true,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn unlock(&self) -> Result<(), Error> {
        if !self.managed {
            return Ok(());
        }
        let password_path = self
            .path
            .parent()
            .ok_or(Error::Invalid)?
            .join("keychain-password");
        let password = match fs::read(&password_path) {
            Ok(password) => password,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.path.exists() {
                    return Err(Error::Unauthorized);
                }
                let mut random = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut random);
                let password = data_encoding::HEXLOWER.encode(&random).into_bytes();
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&password_path)?;
                file.write_all(&password)?;
                file.sync_all()?;
                password
            }
            Err(e) => return Err(e.into()),
        };
        let meta = fs::symlink_metadata(&password_path)?;
        if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 || password.len() != 64 {
            return Err(Error::Unauthorized);
        }
        let password = String::from_utf8(password).map_err(|_| Error::Unauthorized)?;
        let keychain = self.path.to_string_lossy().into_owned();
        if !self.path.exists() {
            let search = security(&["list-keychains", "-d", "user"])?;
            let created = security(&["create-keychain", "-p", &password, &keychain]);
            let mut restore = vec![
                "list-keychains".to_owned(),
                "-d".into(),
                "user".into(),
                "-s".into(),
            ];
            restore.extend(
                String::from_utf8_lossy(&search)
                    .lines()
                    .map(|line| line.trim().trim_matches('"').to_owned())
                    .filter(|line| !line.is_empty()),
            );
            let _ = security(&restore.iter().map(String::as_str).collect::<Vec<_>>());
            created?;
        }
        security(&["unlock-keychain", "-p", &password, &keychain])?;
        Ok(())
    }
    fn legacy(&self, operation: &str, key: &str, value: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let request = serde_json::json!({
            "op": operation,
            "path": self.path,
            "service": self.service,
            "key": key,
            "value": data_encoding::HEXLOWER.encode(value),
        });
        let mut child = Command::new("python3")
            .args(["-c", include_str!("go_keychain.py")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| keychain_error(false))?;
        child
            .stdin
            .take()
            .ok_or_else(|| keychain_error(false))?
            .write_all(request.to_string().as_bytes())?;
        let output = child.wait_with_output()?;
        let result: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| keychain_error(false))?;
        match result["status"].as_i64() {
            Some(0) if operation == "get" => result["value"]
                .as_str()
                .and_then(|text| {
                    data_encoding::HEXLOWER_PERMISSIVE
                        .decode(text.as_bytes())
                        .ok()
                })
                .map(Some)
                .ok_or_else(|| keychain_error(false)),
            Some(0) => Ok(None),
            Some(code) => Err(Self::status(i32::try_from(code).unwrap_or(-1))),
            None => Err(keychain_error(false)),
        }
    }
    fn status(code: i32) -> Error {
        match code {
            ab_sys::keychain::NOT_FOUND => {
                Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound))
            }
            ab_sys::keychain::INTERACTION_NOT_ALLOWED => keychain_error(true),
            _ => keychain_error(false),
        }
    }
}
impl CredentialStore for KeychainStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error> {
        self.unlock()?;
        let encoded = data_encoding::HEXLOWER.encode(bytes);
        match ab_sys::keychain::put(&self.path, &self.service, key, encoded.as_bytes()) {
            Err(ab_sys::keychain::AUTH_FAILED) => self.legacy("put", key, bytes).map(|_| ()),
            other => other.map_err(Self::status),
        }
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        self.unlock()?;
        let encoded = match ab_sys::keychain::get(&self.path, &self.service, key) {
            Err(ab_sys::keychain::AUTH_FAILED) => {
                return self.legacy("get", key, b"")?.ok_or(Error::Unavailable);
            }
            other => other.map_err(Self::status)?,
        };
        data_encoding::HEXLOWER_PERMISSIVE
            .decode(encoded.trim_ascii())
            .map_err(|_| keychain_error(false))
    }
    fn delete(&self, key: &str) -> Result<(), Error> {
        self.unlock()?;
        match ab_sys::keychain::delete(&self.path, &self.service, key) {
            Err(ab_sys::keychain::NOT_FOUND) => Ok(()),
            Err(ab_sys::keychain::AUTH_FAILED) => match self.legacy("delete", key, b"") {
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other.map(|_| ()),
            },
            other => other.map_err(Self::status),
        }
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().ok_or(Error::Invalid)?;
    let mut nonce = [0; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    let temp = parent.join(format!(".write-{}", fingerprint(&nonce)));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temp.exists() {
        fs::remove_file(temp)?;
    }
    result
}
#[cfg(all(test, target_os = "linux"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn environment(root: &Path) -> StoreEnvironment {
        StoreEnvironment {
            secret_store: None,
            xdg_config_home: Some(root.to_string_lossy().into_owned()),
            home: Some(root.to_string_lossy().into_owned()),
            secret_tool: None,
            bundle: None,
        }
    }
    const HOME_DIGEST: &str = "268babf219a745254d0790c2c1983f81c4722a22a017c90c8d5d9bdcd938ba1d";
    const KEY_DIGEST: &str = "a9ffd6f27dd62ca0ee84f59b8c23c25099b9277ba8bafb5078fb4a155d5a35cb";
    #[test]
    fn linux_default_is_the_go_per_home_fingerprint_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = select(
            Path::new("/tmp/ab-g5-home/./nested/.."),
            &environment(root.path()),
        )
        .unwrap();
        store.put("identity-key", b"secret").unwrap();
        let expected = root
            .path()
            .join("apple-bridge/secrets")
            .join(HOME_DIGEST)
            .join(KEY_DIGEST);
        assert_eq!(fs::read(&expected).unwrap(), b"secret");
        assert_eq!(
            fs::metadata(&expected).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(store.get("identity-key").unwrap(), b"secret");
        store.delete("identity-key").unwrap();
        assert!(!expected.exists());
    }
    #[test]
    fn keychain_service_is_the_go_per_home_fingerprint() {
        assert_eq!(
            service_for(Path::new("/tmp/ab-g5-home/./nested/..")),
            format!("apple-bridge:{HOME_DIGEST}")
        );
    }
    #[test]
    fn home_fallback_and_file_override_match_go() {
        let root = tempfile::tempdir().unwrap();
        let mut env = environment(root.path());
        env.xdg_config_home = Some(String::new());
        select(Path::new("/tmp/ab-g5-home"), &env)
            .unwrap()
            .put("identity-key", b"a")
            .unwrap();
        assert!(
            root.path()
                .join(".config/apple-bridge/secrets")
                .join(HOME_DIGEST)
                .join(KEY_DIGEST)
                .exists()
        );
        let home = root.path().join("home");
        env.secret_store = Some("file".into());
        select(&home, &env)
            .unwrap()
            .put("identity-key", b"b")
            .unwrap();
        assert_eq!(
            fs::read(home.join("secrets").join(KEY_DIGEST)).unwrap(),
            b"b"
        );
    }
    #[test]
    fn libsecret_uses_the_go_secret_tool_protocol() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("log");
        let tool = root.path().join("secret-tool");
        fs::write(
            &tool,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$1\" in store) cat > '{log}.value';; lookup) [ -f '{log}.value' ] && cat '{log}.value' || exit 1;; clear) rm -f '{log}.value';; esac\n",
                log = log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = environment(root.path());
        env.secret_tool = Some(tool);
        let store = select(Path::new("/tmp/ab-g5-home"), &env).unwrap();
        assert!(matches!(
            store.get("identity-key"),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
        store.put("identity-key", &[0, 255, 16]).unwrap();
        assert_eq!(store.get("identity-key").unwrap(), vec![0, 255, 16]);
        store.delete("identity-key").unwrap();
        let calls = fs::read_to_string(&log).unwrap();
        assert!(calls.contains(&format!(
            "store --label=Apple Bridge application apple-bridge:{HOME_DIGEST} key identity-key"
        )));
        assert!(calls.contains(&format!(
            "lookup application apple-bridge:{HOME_DIGEST} key identity-key"
        )));
        assert!(fs::read_to_string(format!("{}.value", log.display())).is_err());
    }
}
