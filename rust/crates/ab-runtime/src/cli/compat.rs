use super::Cli;
use clap::CommandFactory;
use std::ffi::{OsStr, OsString};

const TOLERANT_COMMANDS: &[&str] = &[
    "status",
    "doctor",
    "remote",
    "connector",
    "agent",
    "start",
    "pair",
    "join",
    "connect",
    "token",
    "revoke",
    "scope",
    "web",
    "backup",
    "restore",
    "install-credentials",
    "shortcuts",
    "selftest",
    "setup",
];

const BOOLEAN_FLAGS: &[&str] = &[
    "remote",
    "public",
    "iphone",
    "via-remote",
    "helper-lab-commands",
    "accept-ledger-loss",
    "no-ui",
    "force",
    "snippets",
    "private",
];
const VALUE_FLAGS: &[&str] = &[
    "home",
    "host",
    "helper-app",
    "via",
    "name",
    "level",
    "expires-in",
    "request",
    "code",
    "dial-replica",
    "backup",
    "role",
    "listen",
    "api-addr",
    "peer-addr",
    "socket",
    "scope",
    "blob-cap",
    "blob-grace-ms",
    "retention-rows",
    "retention-age",
    "pack",
    "domain",
    "relay-url",
];

fn known(flag: &str) -> bool {
    BOOLEAN_FLAGS.contains(&flag) || VALUE_FLAGS.contains(&flag)
}

enum Rewrite {
    Keep,
    Drop,
    Replace(OsString),
}

fn rewrite(argument: &OsStr) -> Rewrite {
    let Some(text) = argument.to_str() else {
        return Rewrite::Keep;
    };
    let long = if let Some(long) = text.strip_prefix("--") {
        long
    } else if let Some(single) = text.strip_prefix('-')
        && known(single.split_once('=').map_or(single, |(name, _)| name))
    {
        single
    } else {
        return Rewrite::Keep;
    };
    if let Some((name, value)) = long.split_once('=')
        && BOOLEAN_FLAGS.contains(&name)
    {
        return match value {
            "true" => Rewrite::Replace(format!("--{name}").into()),
            "false" => Rewrite::Drop,
            _ => Rewrite::Keep,
        };
    }
    if text.starts_with("--") {
        Rewrite::Keep
    } else {
        Rewrite::Replace(format!("--{long}").into())
    }
}

fn declared_flags(arguments: &[OsString]) -> Option<Vec<String>> {
    let root = Cli::command();
    let verb = arguments.get(1)?.to_str()?;
    if !TOLERANT_COMMANDS.contains(&verb) {
        return None;
    }
    let mut command = root.find_subcommand(verb)?;
    if let Some(nested) = arguments
        .get(2)
        .and_then(|action| action.to_str())
        .and_then(|action| command.find_subcommand(action))
    {
        command = nested;
    }
    let mut flags: Vec<String> = command
        .get_arguments()
        .filter_map(|argument| argument.get_long().map(str::to_owned))
        .collect();
    flags.push("home".to_owned());
    Some(flags)
}

fn without_ignored_flags(arguments: Vec<OsString>) -> Vec<OsString> {
    let Some(declared) = declared_flags(&arguments) else {
        return arguments;
    };
    let mut output = Vec::with_capacity(arguments.len());
    let mut remaining = arguments.into_iter();
    while let Some(argument) = remaining.next() {
        let name = argument
            .to_str()
            .and_then(|text| text.strip_prefix("--"))
            .map(|long| {
                long.split_once('=')
                    .map_or(long, |(name, _)| name)
                    .to_owned()
            });
        let has_value = argument.to_str().is_some_and(|text| text.contains('='));
        match name {
            Some(name) if known(&name) && !declared.contains(&name) => {
                if VALUE_FLAGS.contains(&name.as_str()) && !has_value {
                    remaining.next();
                }
            }
            _ => output.push(argument),
        }
    }
    output
}

pub fn normalize(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut output = Vec::new();
    for argument in arguments {
        match rewrite(&argument) {
            Rewrite::Keep => output.push(argument),
            Rewrite::Drop => {}
            Rewrite::Replace(replacement) => output.push(replacement),
        }
    }
    without_ignored_flags(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arguments: &[&str]) -> Vec<String> {
        normalize(arguments.iter().map(OsString::from))
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn single_dash_long_flags_keep_working() {
        assert_eq!(
            run(&["bridge", "status", "-home", "/h"]),
            ["bridge", "status", "--home", "/h"]
        );
        assert_eq!(
            run(&["bridge", "status", "-home=/h"]),
            ["bridge", "status", "--home=/h"]
        );
    }

    #[test]
    fn booleans_with_values_become_plain_flags() {
        assert_eq!(
            run(&["bridge", "pair", "--iphone=true", "--via-remote=false"]),
            ["bridge", "pair", "--iphone"]
        );
    }

    #[test]
    fn older_commands_ignore_flags_they_never_used_and_new_ones_stay_strict() {
        assert_eq!(
            run(&[
                "bridge", "pair", "--name", "r", "--scope", "a:read", "--force"
            ]),
            ["bridge", "pair", "--scope", "a:read"]
        );
        assert_eq!(
            run(&["bridge", "agent", "create", "--name", "n", "--via=x"]),
            ["bridge", "agent", "create", "--name", "n"]
        );
        assert_eq!(
            run(&["bridge", "up", "--force", "--name", "r"]),
            ["bridge", "up", "--force", "--name", "r"]
        );
    }

    #[test]
    fn everything_else_is_untouched() {
        let unchanged = [
            "bridge",
            "-h",
            "--help",
            "-V",
            "--json",
            "-x",
            "--force",
            "--force=maybe",
            "value",
        ];
        assert_eq!(run(&unchanged), unchanged);
    }
}
