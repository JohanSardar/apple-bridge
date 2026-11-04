use crate::{Error, Result, World, stop};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const READY: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub(crate) listen: Option<PathBuf>,
    pub(crate) gateway: String,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) native: Option<PathBuf>,
    pub(crate) facts: Option<PathBuf>,
    pub(crate) state: PathBuf,
    pub(crate) roots: Vec<(String, PathBuf)>,
}

fn variable(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

pub fn session_binary() -> Option<PathBuf> {
    Some(PathBuf::from(variable("AB_LAB_SESSION"))).filter(|path| !path.as_os_str().is_empty())
}

pub fn session_sources() -> Vec<String> {
    if session_binary().is_none() {
        return Vec::new();
    }
    let mut sources: Vec<String> = variable("AB_LAB_SESSION_SOURCES")
        .split(',')
        .map(|source| source.trim().to_owned())
        .filter(|source| !source.is_empty())
        .collect();
    sources.sort();
    sources
}

pub fn session_effects() -> bool {
    session_binary().is_some() && variable("AB_LAB_SESSION_EFFECTS") == "1"
}

impl World {
    pub(crate) fn helper_socket(&self, gateway: &str) -> String {
        self.labsession
            .listen
            .as_ref()
            .map_or_else(|| gateway.to_owned(), |path| path.display().to_string())
    }

    pub(crate) fn sim_native_socket(&self) -> Option<PathBuf> {
        self.labsession
            .native
            .clone()
            .or_else(|| self.labsession.facts.clone())
    }

    pub(crate) fn start_session(&mut self, gateway: &str) -> Result<()> {
        let sources = session_sources();
        let effects = session_effects();
        if self.labsession.listen.is_some() || (sources.is_empty() && !effects) {
            return Ok(());
        }
        let facts_socket = match variable("AB_LAB_SESSION_NATIVE_FACTS").as_str() {
            "1" => Some(self.directory.join("native.sock")),
            "missing" => Some(self.directory.join("native-facts-missing.sock")),
            _ => None,
        };
        if variable("AB_LAB_SESSION_NATIVE_FACTS") == "1" && !effects {
            self.labsession.facts.clone_from(&facts_socket);
        }
        let mut producers = serde_json::Map::new();
        let mut config = serde_json::Map::new();
        for source in &sources {
            let root = self.directory.join(format!("session-{source}"));
            fs::create_dir_all(&root)?;
            producers.insert(source.clone(), json!("Rust"));
            let mut entry = json!({"root": root});
            if let Some(socket) = &facts_socket
                && matches!(source.as_str(), "calendar" | "contacts" | "reminders")
            {
                entry["facts"] = json!({"socket": socket});
                entry["facts_poll_ms"] = json!(3_600_000);
            }
            if matches!(source.as_str(), "notes" | "voice_memos") {
                let staging = self.directory.join(format!("session-staging-{source}"));
                fs::create_dir_all(&staging)?;
                entry["staging"] = json!(staging);
            }
            if source == "notes" {
                entry["hydration_hold"] = json!(self.directory.join("session-notes-hold"));
            }
            if source == "usage" {
                entry["local_name"] = json!("Synthetic mac local");
            }
            self.labsession.roots.push((source.clone(), root));
            config.insert(source.clone(), entry);
        }
        self.labsession.state = self.directory.join("session-state");
        self.labsession.env = vec![
            (
                "AB_SESSION_PRODUCERS".into(),
                serde_json::to_string(&Value::Object(producers))?,
            ),
            (
                "AB_SESSION_RUST_SOURCES".into(),
                serde_json::to_string(&Value::Object(config))?,
            ),
            (
                "AB_SESSION_RUST_STATE".into(),
                self.labsession.state.display().to_string(),
            ),
        ];
        if effects {
            self.labsession.native = Some(self.directory.join("native.sock"));
            self.labsession.env.push((
                "AB_SESSION_EFFECTS_HOME".into(),
                self.directory.join("session-effects").display().to_string(),
            ));
        }
        self.labsession.listen = Some(self.directory.join("session.sock"));
        gateway.clone_into(&mut self.labsession.gateway);
        self.spawn_session()
    }

    fn spawn_session(&mut self) -> Result<()> {
        let binary =
            session_binary().ok_or_else(|| Error::Invalid("AB_LAB_SESSION is not set".into()))?;
        let listen = self
            .labsession
            .listen
            .clone()
            .ok_or_else(|| Error::Invalid("session socket".into()))?;
        let _ = fs::remove_file(&listen);
        let log = fs::File::options()
            .create(true)
            .append(true)
            .open(self.directory.join("session.log"))?;
        let mut command = Command::new(binary);
        command
            .current_dir(&self.directory)
            .arg("--lab-listen")
            .arg(&listen)
            .args(["--lab-gateway", &self.labsession.gateway])
            .envs(self.labsession.env.iter().map(|(k, v)| (k, v)))
            .stdout(log.try_clone()?)
            .stderr(log);
        if let Some(native) = &self.labsession.native {
            command.arg("--lab-native").arg(native);
        }
        self.labsession_child = Some(command.spawn()?);
        let deadline = Instant::now() + READY;
        while !listen.exists() {
            if let Some(status) = self
                .labsession_child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
            {
                return Err(Error::Invalid(format!("session exited {status}")));
            }
            if Instant::now() >= deadline {
                return Err(Error::Invalid("session readiness timeout".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }

    pub fn session_root(&self, source: &str) -> Option<&Path> {
        self.labsession
            .roots
            .iter()
            .find(|(name, _)| name == source)
            .map(|(_, root)| root.as_path())
    }

    #[must_use]
    pub fn session_checkpoint(&self, source: &str) -> String {
        fs::read(self.labsession.state.join(format!("{source}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|record| record["checkpoint"].as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    pub fn restart_session(&mut self) -> Result<()> {
        if self.labsession.listen.is_none() {
            return Err(Error::Invalid("no session".into()));
        }
        stop(&mut self.labsession_child);
        self.spawn_session()
    }

    pub(crate) fn keep_session_evidence(&self) {
        let keep = variable("AB_LAB_SESSION_EFFECTS_EVIDENCE");
        if keep.is_empty() || self.labsession.native.is_none() {
            return;
        }
        let Some(name) = self.directory.file_name() else {
            return;
        };
        let destination = Path::new(&keep).join(name);
        if fs::create_dir_all(&destination).is_err() {
            return;
        }
        let copies = [
            (
                self.directory.join("session-effects/dispatch-guard.log"),
                "dispatch-guard.log",
            ),
            (self.directory.join("session.log"), "session.log"),
            (self.directory.join("simapple.log"), "simapple.log"),
            (self.directory.join("gateway.log"), "gateway.log"),
        ];
        for (from, to) in copies {
            if let Ok(bytes) = fs::read(from) {
                let _ = fs::write(destination.join(to), bytes);
            }
        }
    }
}
