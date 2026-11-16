use std::{
    io::IsTerminal,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

const AGENT_VARIABLES: &[&str] = &[
    "AI_AGENT",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CODEX_SANDBOX",
    "CODEX_THREAD_ID",
    "CURSOR_AGENT",
    "GEMINI_CLI",
    "CLINE_ACTIVE",
    "OPENCODE",
    "AMP_AGENT",
];

#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Requested {
    pub json: bool,
    pub non_interactive: Option<bool>,
    pub yes: bool,
    pub no_color: bool,
    pub debug: bool,
    pub explicit_home: Option<PathBuf>,
    pub explicit_cwd: Option<PathBuf>,
    pub fields: Option<Vec<String>>,
    pub accessible: bool,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct Modes {
    pub json: bool,
    pub yes: bool,
    pub debug: bool,
    pub fields: Option<Vec<String>>,
    pub explicit_home: Option<PathBuf>,
    pub explicit_cwd: Option<PathBuf>,
    pub accessible: bool,
    pub ci: bool,
    pub agent: Option<String>,
    pub github_actions: bool,
    non_interactive: Option<bool>,
    stdin_tty: bool,
    stdout_tty: bool,
    stderr_tty: bool,
    dumb: bool,
    color: bool,
    stdout_color: bool,
    width: usize,
}

static MODES: OnceLock<Modes> = OnceLock::new();
static TYPED: OnceLock<Vec<String>> = OnceLock::new();

pub fn set_typed(words: &[String]) {
    let _ = TYPED.set(words.to_vec());
}

pub fn legacy_spelling() -> bool {
    let words = TYPED.get().map(Vec::as_slice).unwrap_or_default();
    let first = words.first().map_or("", String::as_str);
    let second = words.get(1).map_or("", String::as_str);
    matches!(
        first,
        "agent" | "token" | "connector" | "revoke" | "scope" | "setup"
    ) || matches!(
        (first, second),
        ("web", "devices" | "lock") | ("connectors", "pending") | ("agents", "level")
    )
}
static STDIN_PAYLOAD: AtomicBool = AtomicBool::new(false);

fn set(name: &str) -> bool {
    std::env::var_os(name)
        .is_some_and(|value| !value.is_empty() && value != "0" && value != "false")
}

impl Modes {
    pub fn resolve(requested: Requested) -> Self {
        let stdin_tty = std::io::stdin().is_terminal();
        let stdout_tty = std::io::stdout().is_terminal();
        let stderr_tty = std::io::stderr().is_terminal();
        let accessible = requested.accessible || set("APPLE_BRIDGE_ACCESSIBLE");
        let dumb = accessible || std::env::var("TERM").is_ok_and(|term| term == "dumb");
        let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        let forced = std::env::var("FORCE_COLOR").ok();
        let refused = requested.no_color || no_color_env || accessible;
        let forced = forced
            .as_deref()
            .filter(|force| !force.is_empty())
            .map(|force| force != "0");
        let color = !refused && forced.unwrap_or(stderr_tty && !dumb);
        let stdout_color = !refused && forced.unwrap_or(stdout_tty && !dumb);
        let width = std::env::var("COLUMNS")
            .ok()
            .and_then(|columns| columns.parse::<usize>().ok())
            .filter(|columns| *columns >= 20)
            .or_else(|| ab_sys::terminal_columns(2).map(usize::from))
            .unwrap_or(80);
        Self {
            json: requested.json,
            yes: requested.yes,
            debug: requested.debug,
            fields: requested.fields,
            explicit_home: requested.explicit_home,
            explicit_cwd: requested
                .explicit_cwd
                .map(|cwd| std::path::absolute(&cwd).unwrap_or(cwd)),
            accessible,
            ci: set("CI"),
            agent: AGENT_VARIABLES
                .iter()
                .find(|name| set(name))
                .map(|name| (*name).to_owned()),
            github_actions: std::env::var("GITHUB_ACTIONS").is_ok_and(|value| value == "true"),
            non_interactive: requested.non_interactive,
            stdin_tty,
            stdout_tty,
            stderr_tty,
            dumb,
            color,
            stdout_color,
            width,
        }
    }

    pub fn install(self) {
        let _ = MODES.set(self);
    }

    pub fn get() -> &'static Self {
        MODES.get_or_init(|| Self::resolve(Requested::default()))
    }

    pub fn interactive(&self) -> bool {
        if self.json
            || self.non_interactive == Some(true)
            || self.ci
            || STDIN_PAYLOAD.load(Ordering::Relaxed)
            || !self.stdin_tty
            || !self.stderr_tty
        {
            return false;
        }
        self.agent.is_none() || self.non_interactive == Some(false)
    }

    pub fn why_not_interactive(&self) -> &'static str {
        if self.json {
            "with --json"
        } else if self.non_interactive == Some(true) {
            "with --non-interactive"
        } else if self.ci {
            "in CI"
        } else if self.agent.is_some() && self.non_interactive != Some(false) {
            "when run by an agent"
        } else if STDIN_PAYLOAD.load(Ordering::Relaxed) {
            "while stdin carries the payload"
        } else {
            "without a terminal"
        }
    }

    pub fn stdout_tty(&self) -> bool {
        self.stdout_tty
    }

    pub fn rich(&self) -> bool {
        self.stderr_tty && !self.dumb
    }

    pub fn color(&self) -> bool {
        self.color
    }

    pub fn stdout_styled(&self) -> bool {
        self.stdout_color && self.stdout_tty && !self.dumb
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn stdout_table(&self) -> bool {
        self.stdout_tty && !self.dumb
    }

    pub fn legacy_json(&self) -> bool {
        !self.json
            && !self.stdout_tty
            && std::env::var("APPLE_BRIDGE_LEGACY_JSON").map_or(true, |value| value != "0")
    }

    pub fn browser_allowed(&self) -> bool {
        self.interactive()
            && std::env::var_os("SSH_CONNECTION").is_none()
            && std::env::var_os("SSH_TTY").is_none()
    }
}

pub fn mark_stdin_payload() {
    STDIN_PAYLOAD.store(true, Ordering::Relaxed);
}
