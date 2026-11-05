use crate::Error;
use std::path::Path;

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub next: Option<String>,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            next: None,
        }
    }

    #[must_use]
    pub fn next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }

    pub fn not_set_up(home: &Path) -> Self {
        Self::new(format!(
            "Apple Bridge has not been set up in {}.",
            home.display()
        ))
        .next("Run `bridge up` first, or pass --home with the folder you use.")
    }

    pub fn not_running(home: &Path) -> Self {
        Self::new(format!(
            "The gateway for {} is not running.",
            home.display()
        ))
        .next("Run `bridge up`.")
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Locked => Self::new("Another gateway is already running from this home.")
                .next("Run `bridge stats` to use it, or `bridge down` to stop it."),
            Error::Io(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                Self::new("That address is already in use by another program.")
                    .next("Choose a free one with --api-addr 127.0.0.1:PORT.")
            }
            Error::Invalid => {
                Self::new("This home's settings are missing or do not fit that command.")
                    .next("Run `bridge doctor` to check it, or `bridge --help` for usage.")
            }
            other => Self::new(other.to_string()),
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
        Self::from(Error::from(error))
    }
}

impl From<ab_remote::Error> for Failure {
    fn from(error: ab_remote::Error) -> Self {
        Self::from(Error::from(error))
    }
}

pub type Outcome<T = ()> = Result<T, Failure>;
