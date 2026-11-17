use crate::Error;
use serde_json::{Map, Value, json};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Usage,
    MissingInput,
    Permission,
    NotFound,
    Conflict,
    Network,
    Platform,
    UnknownOutcome,
    Config,
    Bug,
    Canceled,
}

macro_rules! reasons {
    ($($variant:ident => $text:literal, $class:ident;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Reason { $($variant),* }

        impl Reason {
            pub const ALL: &'static [Reason] = &[$(Reason::$variant),*];

            pub fn as_str(self) -> &'static str {
                match self { $(Reason::$variant => $text),* }
            }

            pub fn class(self) -> Class {
                match self { $(Reason::$variant => Class::$class),* }
            }
        }
    };
}

reasons! {
    InvalidArguments => "invalid_arguments", Usage;
    UnknownOption => "unknown_option", Usage;
    UnknownCommand => "unknown_command", Usage;
    MissingArgument => "missing_argument", Usage;
    InvalidValue => "invalid_value", Usage;
    ConflictingOptions => "conflicting_options", Usage;
    InvalidHome => "invalid_home", Usage;
    MissingInput => "missing_input", MissingInput;
    ConfirmationRequired => "confirmation_required", MissingInput;
    TerminalRequired => "terminal_required", MissingInput;
    MissingTimeout => "missing_timeout", MissingInput;
    NotRunning => "not_running", Conflict;
    NotSetUp => "not_set_up", NotFound;
    NotFound => "not_found", NotFound;
    SourceUnavailable => "source_unavailable", Conflict;
    PermissionMissing => "permission_missing", Permission;
    PermissionDenied => "permission_denied", Permission;
    ReadOnly => "read_only", Permission;
    Conflict => "conflict", Conflict;
    DestinationExists => "destination_exists", Conflict;
    PortInUse => "port_in_use", Conflict;
    GatewayBusy => "gateway_busy", Conflict;
    NotOwned => "not_owned", Conflict;
    LedgerMissing => "ledger_missing", Conflict;
    NetworkError => "network_error", Network;
    RateLimited => "rate_limited", Network;
    PaginationStalled => "pagination_stalled", Platform;
    ApiError => "api_error", Platform;
    SpawnFailed => "spawn_failed", Platform;
    MissingCommand => "missing_command", Platform;
    HelperFailed => "helper_failed", Platform;
    UnhealthyHome => "unhealthy_home", Platform;
    Unproven => "unproven", Platform;
    UnknownOutcome => "unknown_outcome", UnknownOutcome;
    Timeout => "timeout", UnknownOutcome;
    InvalidConfig => "invalid_config", Config;
    InternalError => "internal_error", Bug;
    Canceled => "canceled", Canceled;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub benefit: String,
    pub argv: Vec<String>,
    pub when: Option<String>,
}

impl Step {
    pub fn new<S: Into<String>>(
        benefit: impl Into<String>,
        argv: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            benefit: benefit.into(),
            argv: argv
                .into_iter()
                .map(|argument| super::ui::strip_userinfo(&argument.into()))
                .collect(),
            when: None,
        }
    }

    #[must_use]
    pub fn when(mut self, when: impl Into<String>) -> Self {
        self.when = Some(when.into());
        self
    }

    pub fn full_argv(&self) -> Vec<String> {
        let mut argv = self.argv.clone();
        let modes = super::modes::Modes::get();
        if argv.first().is_some_and(|program| program == "bridge") {
            if let Some(home) = &modes.explicit_home
                && !argv.iter().any(|argument| argument == "--home")
            {
                argv.push("--home".to_owned());
                argv.push(home.to_string_lossy().into_owned());
            }
            if let Some(cwd) = &modes.explicit_cwd
                && !argv.iter().any(|argument| argument == "--cwd")
            {
                argv.push("--cwd".to_owned());
                argv.push(cwd.to_string_lossy().into_owned());
            }
        }
        argv
    }

    pub fn command(&self) -> String {
        super::ui::shell_line(&self.full_argv())
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({"command": self.command(), "argv": self.full_argv()});
        if let Some(when) = &self.when {
            value["when"] = json!(when);
        }
        value
    }
}

#[derive(Debug, Clone)]
pub struct Failure(Box<Details>);

#[derive(Debug, Clone)]
pub struct Details {
    pub reason: Reason,
    pub headline: String,
    pub rows: Vec<(String, String)>,
    pub remote: Option<(String, String)>,
    pub issues: Vec<String>,
    pub next: Vec<Step>,
    pub sentence: Option<String>,
    pub data: Map<String, Value>,
    pub action_required: Option<bool>,
    pub quiet_in_json: bool,
}

impl std::ops::Deref for Failure {
    type Target = Details;

    fn deref(&self) -> &Details {
        &self.0
    }
}

impl std::ops::DerefMut for Failure {
    fn deref_mut(&mut self) -> &mut Details {
        &mut self.0
    }
}

impl Failure {
    pub fn new(reason: Reason, headline: impl Into<String>) -> Self {
        Self(Box::new(Details {
            reason,
            headline: super::ui::strip_userinfo(&headline.into()),
            rows: Vec::new(),
            remote: None,
            issues: Vec::new(),
            next: Vec::new(),
            sentence: None,
            data: Map::new(),
            action_required: None,
            quiet_in_json: false,
        }))
    }

    pub fn usage(headline: impl Into<String>) -> Self {
        Self::new(Reason::InvalidArguments, headline)
    }

    pub fn invalid(headline: impl Into<String>) -> Self {
        Self::new(Reason::InvalidValue, headline)
    }

    #[must_use]
    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.rows
            .push((label.into(), super::ui::strip_userinfo(&value.into())));
        self
    }

    #[must_use]
    pub fn remote(mut self, source: impl Into<String>, text: impl Into<String>) -> Self {
        self.remote = Some((source.into(), super::ui::redact_urls(&text.into())));
        self
    }

    #[must_use]
    pub fn issue(mut self, issue: impl Into<String>) -> Self {
        self.issues.push(super::ui::strip_userinfo(&issue.into()));
        self
    }

    #[must_use]
    pub fn step(mut self, step: Step) -> Self {
        self.next.push(step);
        self
    }

    #[must_use]
    pub fn then<S: Into<String>>(
        self,
        benefit: impl Into<String>,
        argv: impl IntoIterator<Item = S>,
    ) -> Self {
        self.step(Step::new(benefit, argv))
    }

    #[must_use]
    pub fn sentence(mut self, sentence: impl Into<String>) -> Self {
        self.sentence = Some(super::ui::strip_userinfo(&sentence.into()));
        self
    }

    #[must_use]
    pub fn data(mut self, key: &str, value: impl Into<Value>) -> Self {
        let mut value = value.into();
        super::ui::strip_userinfo_json(&mut value);
        self.data.insert(key.to_owned(), value);
        self
    }

    #[must_use]
    pub fn quiet_in_json(mut self) -> Self {
        self.quiet_in_json = true;
        self
    }

    #[must_use]
    pub fn needs_action(mut self) -> Self {
        self.action_required = Some(true);
        self
    }

    pub fn exit_code(&self) -> u8 {
        match self.reason.class() {
            Class::Usage | Class::MissingInput => 2,
            Class::Canceled => 130,
            _ => 1,
        }
    }

    pub fn status(&self) -> &'static str {
        let blocked = self.action_required.unwrap_or(
            matches!(
                self.reason,
                Reason::NotRunning
                    | Reason::NotSetUp
                    | Reason::PermissionMissing
                    | Reason::SourceUnavailable
            ) || self.reason.class() == Class::MissingInput,
        );
        if blocked { "action_required" } else { "error" }
    }

    pub fn to_json(&self, warnings: &[(String, String)]) -> Value {
        let mut document = json!({
            "status": self.status(),
            "reason": self.reason.as_str(),
            "message": self.headline,
        });
        if !self.next.is_empty() {
            document["next"] = Value::Array(self.next.iter().map(Step::to_json).collect());
        }
        let mut data = self.data.clone();
        if let Some((source, text)) = &self.remote {
            data.insert("remoteSource".into(), json!(source));
            data.insert("remoteMessage".into(), json!(super::ui::clean(text)));
        }
        if !self.issues.is_empty() {
            data.insert(
                "issues".into(),
                Value::Array(
                    self.issues
                        .iter()
                        .map(|issue| json!({"message": issue}))
                        .collect(),
                ),
            );
        }
        if !self.rows.is_empty() && !data.contains_key("effects") {
            for (label, value) in &self.rows {
                let key = label.to_lowercase().replace(' ', "_");
                data.entry(key).or_insert_with(|| json!(value));
            }
        }
        if !data.is_empty() {
            document["data"] = Value::Object(data);
        }
        if self.status() == "action_required" && self.reason == Reason::ConfirmationRequired {
            document["userActionRequired"] = json!(true);
        }
        if !warnings.is_empty() {
            document["warnings"] = Value::Array(
                warnings
                    .iter()
                    .map(|(reason, message)| json!({"reason": reason, "message": message}))
                    .collect(),
            );
        }
        document
    }

    pub fn not_set_up(home: &Path) -> Self {
        Self::new(
            Reason::NotSetUp,
            format!("Apple Bridge isn't set up in {}.", home.display()),
        )
        .then("Set it up", ["bridge", "up"])
    }

    pub fn missing_home(home: &Path) -> Self {
        let shown = super::commands::absolute(home);
        Self::new(
            Reason::NotSetUp,
            format!("Apple Bridge isn't set up in {}.", shown.display()),
        )
        .then(
            "Set it up",
            [
                "bridge".to_owned(),
                "up".to_owned(),
                "--home".to_owned(),
                shown.to_string_lossy().into_owned(),
            ],
        )
    }

    pub fn not_running(home: &Path) -> Self {
        Self::new(
            Reason::NotRunning,
            format!("Apple Bridge isn't running from {}.", home.display()),
        )
        .then("Start it", ["bridge", "up"])
    }

    pub fn canceled(note: impl Into<String>) -> Self {
        Self::new(Reason::Canceled, note)
    }

    pub fn internal(error: &impl std::fmt::Display) -> Self {
        Self::new(
            Reason::InternalError,
            "Unexpected error in bridge. This is a bug in Apple Bridge.",
        )
        .row("Error", super::ui::clean(&error.to_string()))
        .then(
            "Review the --debug output, then report it",
            ["https://github.com/JohanSardar/apple-bridge/issues/new"],
        )
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Locked => Self::new(
                Reason::GatewayBusy,
                "Can't use this home. Another gateway is running from it.",
            )
            .then("Use the running gateway", ["bridge", "stats"])
            .then("Or stop it", ["bridge", "down"]),
            Error::Io(error) if error.kind() == std::io::ErrorKind::AddrInUse => Self::new(
                Reason::PortInUse,
                "Can't start the gateway. Its address is in use by another program.",
            )
            .then(
                "Choose a free address",
                ["bridge", "up", "--api-addr", "127.0.0.1:<PORT>"],
            ),
            Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Self::new(
                Reason::PermissionDenied,
                "Can't use this home. A file in it isn't readable or writable by you.",
            )
            .row("Detail", super::ui::clean(&error.to_string()))
            .then("Check it", ["bridge", "doctor"]),
            Error::Invalid => Self::new(
                Reason::InvalidConfig,
                "Can't use this home. Its settings are missing or don't fit this command.",
            )
            .then("Check the home", ["bridge", "doctor"]),
            other => Self::internal(&other),
        }
    }
}

impl From<ab_trust::Error> for Failure {
    fn from(error: ab_trust::Error) -> Self {
        Self::from(Error::from(error))
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::from(Error::from(error))
    }
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(&error)
    }
}

impl From<ab_remote::Error> for Failure {
    fn from(error: ab_remote::Error) -> Self {
        let text = error.to_string();
        Self::new(Reason::ApiError, "Failed to change Tailscale access.")
            .remote("Tailscale", text)
            .then("Check it", ["bridge", "status"])
    }
}

pub type Outcome<T = ()> = Result<T, Failure>;
