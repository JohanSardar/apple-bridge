use crate::{Error, Result, World, stop};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub(crate) gateway: String,
    pub(crate) facts: Option<PathBuf>,
    pub(crate) state: PathBuf,
    pub(crate) roots: Vec<(String, PathBuf)>,
    pub(crate) usage: std::sync::Mutex<crate::session_fixtures::Devices>,
    pub(crate) mail: std::sync::Mutex<crate::session_fixtures::Mail>,
    pub(crate) notes: Option<crate::session_notes::NotesSync>,
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

pub(crate) fn native_session() -> bool {
    session_binary().is_some() || !matches!(variable("AB_LAB_SESSION_NATIVE").as_str(), "" | "0")
}

impl World {
    pub(crate) fn session_hook(&self, route: &str, input: &Value) -> Result<()> {
        match route {
            "/usage" if self.session_root("usage").is_some() => self.stage_session_usage(input),
            "/dataset" if self.session_root("mail").is_some() => self.stage_session_mail(input),
            _ => Ok(()),
        }
    }

    pub(crate) fn session_after(&self, route: &str) {
        if route == "/external-change"
            && let Some(notes) = &self.labsession.notes
            && let Err(error) = notes.sync()
        {
            eprintln!("lab session notes: {error}");
        }
    }

    pub(crate) fn start_session_notes(&mut self) {
        if self.labsession.notes.is_some() {
            return;
        }
        if let Some(root) = self.session_root("notes").map(Path::to_path_buf) {
            self.labsession.notes = Some(crate::session_notes::NotesSync::start(
                self.directory.clone(),
                root,
                self.directory.join("session-notes-hold"),
            ));
        }
    }

    pub(crate) fn stop_session_notes(&mut self) {
        if let Some(notes) = self.labsession.notes.as_mut() {
            notes.stop();
        }
    }

    pub(crate) fn sim_native_socket(&self) -> Option<PathBuf> {
        self.labsession.facts.clone()
    }

    pub(crate) fn start_session(&mut self, gateway: &str) -> Result<()> {
        let sources = session_sources();
        if self.labsession.state.as_os_str().is_empty() && sources.is_empty() {
            return Ok(());
        }
        if !self.labsession.state.as_os_str().is_empty() {
            return Ok(());
        }
        let facts_socket = match variable("AB_LAB_SESSION_NATIVE_FACTS").as_str() {
            "1" => Some(self.directory.join("native.sock")),
            "missing" => Some(self.directory.join("native-facts-missing.sock")),
            _ => None,
        };
        if variable("AB_LAB_SESSION_NATIVE_FACTS") == "1" {
            self.labsession.facts.clone_from(&facts_socket);
        }
        let mut config = serde_json::Map::new();
        for source in &sources {
            let root = self.directory.join(format!("session-{source}"));
            fs::create_dir_all(&root)?;
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
        gateway.clone_into(&mut self.labsession.gateway);
        let home = self.directory.join("session-home");
        fs::create_dir_all(home.join("session"))?;
        fs::write(
            home.join("session/rust-capture.json"),
            serde_json::to_vec(&json!({
                "sources": Value::Object(config),
                "watch": variable("AB_SESSION_RUST_WATCH") == "1",
                "poll_ms": 100,
            }))?,
        )?;
        self.labsession.state = home.join("session/capture-state");
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
        if self.labsession.state.as_os_str().is_empty() {
            return Err(Error::Invalid("no session".into()));
        }
        stop(&mut self.session);
        let gateway = self.labsession.gateway.clone();
        self.launch_session(&gateway)?;
        Ok(())
    }
}
