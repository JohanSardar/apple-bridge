use crate::{
    Error,
    cli::{Input, Runner},
    clock::valid_rfc3339,
};
use serde::{Deserialize, Serialize};
use std::{thread, time::Duration};

pub const NAME: &str = "AB Focus";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub focused: Option<bool>,
    pub name: Option<String>,
    pub authorization: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetArgs {
    pub mode: String,
    pub on: bool,
    pub until: String,
}

pub type ReadStatus<'a> = &'a dyn Fn() -> Result<Status, Error>;

pub struct Pack<'a> {
    pub runner: Option<&'a dyn Runner>,
    pub read_status: Option<ReadStatus<'a>>,
}

impl std::fmt::Debug for Pack<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack").finish_non_exhaustive()
    }
}

fn unqualified() -> Error {
    Error::FocusUnqualified
}

fn future(until: &str) -> bool {
    valid_rfc3339(until)
        && until
            .parse::<jiff::Timestamp>()
            .is_ok_and(|t| t > jiff::Timestamp::now())
}

impl Pack<'_> {
    pub fn set(&self, args: &SetArgs) -> Result<(), Error> {
        let (Some(runner), Some(read_status)) = (self.runner, self.read_status) else {
            return Err(unqualified());
        };
        if args.mode != "Do Not Disturb" {
            return Err(unqualified());
        }
        if !args.until.is_empty() && (!args.on || !future(&args.until)) {
            return Err(unqualified());
        }
        let before = read_status()?;
        if before.authorization != "authorized" || before.focused.is_none() {
            return Err(unqualified());
        }
        let result = runner.run(&Input {
            shortcut: NAME.into(),
            data: Some(serde_json::to_vec(args)?),
            timeout: Duration::from_secs(15),
            ..Input::default()
        })?;
        if result.timed_out || result.exit_status != 0 {
            return Err(unqualified());
        }
        for _ in 0..20 {
            let status = read_status()?;
            if status.authorization == "authorized" && status.focused == Some(args.on) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(200));
        }
        Err(unqualified())
    }
}
