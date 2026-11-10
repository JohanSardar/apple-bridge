use super::Cli;
use clap::CommandFactory;
use std::ffi::OsString;

const TOLERANT_COMMANDS: &[&str] = &[
    "status",
    "doctor",
    "remote",
    "connector",
    "connectors",
    "agent",
    "agents",
    "start",
    "pair",
    "join",
    "connect",
    "token",
    "tokens",
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

const LEGACY_BOOLEANS: &[&str] = &[
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

const LEGACY_VALUES: &[&str] = &[
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

const CONSEQUENTIAL: &[&str] = &[
    "snippets",
    "force",
    "accept-ledger-loss",
    "remote",
    "public",
    "scope",
    "level",
    "via",
    "private",
    "domain",
    "relay-url",
    "backup",
    "iphone",
    "via-remote",
    "dial-replica",
    "peer-addr",
    "api-addr",
    "listen",
    "host",
    "socket",
    "role",
];

const READ_ONLY_LEAVES: &[&str] = &[
    "status",
    "doctor",
    "remote status",
    "connectors list",
    "connectors pending",
    "agents list",
    "agents inspect",
    "web sessions list",
    "web devices",
    "selftest",
    "shortcuts",
    "remote",
    "agents",
    "tokens",
    "connectors",
    "web",
    "web sessions",
    "remote certificates",
];

const GLOBAL_VALUES: &[&str] = &["home", "fields", "cwd"];

const REMOTE_VALUES: &[&str] = &["via", "domain", "relay-url", "name", "timeout"];

#[derive(Debug, Default)]
pub struct Prepared {
    pub arguments: Vec<OsString>,
    pub path: Vec<String>,
    pub typed: Vec<String>,
    pub help: bool,
    pub version: bool,
    pub notices: Vec<(String, String)>,
    pub stray: Option<String>,
}

fn known(flag: &str) -> bool {
    LEGACY_BOOLEANS.contains(&flag) || LEGACY_VALUES.contains(&flag)
}

pub fn resolve_path(tokens: &[String]) -> (Vec<String>, clap::Command) {
    let (path, _, command, _) = resolve(tokens);
    (path, command)
}

fn resolve(tokens: &[String]) -> (Vec<String>, Vec<String>, clap::Command, Option<String>) {
    let mut command = Cli::command();
    let mut path = Vec::new();
    let mut typed = Vec::new();
    let mut stray = None;
    let mut skip = false;
    for token in tokens {
        if skip {
            skip = false;
            continue;
        }
        if token == "--" {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            let name = long.split_once('=').map_or(long, |(name, _)| name);
            if !long.contains('=') && takes_value(&command, name) {
                skip = true;
            }
            continue;
        }
        if token.starts_with('-') && token.len() > 1 {
            continue;
        }
        let Some(child) = command
            .get_subcommands()
            .find(|child| {
                child.get_name() == token || child.get_all_aliases().any(|alias| alias == token)
            })
            .cloned()
        else {
            if command.has_subcommands() {
                stray = Some(token.clone());
            }
            break;
        };
        path.push(child.get_name().to_owned());
        typed.push(token.clone());
        command = child;
    }
    (path, typed, command, stray)
}

fn option_name(token: &str) -> Option<&str> {
    let long = token.strip_prefix("--")?;
    Some(long.split_once('=').map_or(long, |(name, _)| name))
}

fn hoist_remote_options(arguments: Vec<OsString>) -> Vec<OsString> {
    let tokens: Vec<String> = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            return arguments;
        }
        if let Some(name) = option_name(token) {
            index += if !token.contains('=') && GLOBAL_VALUES.contains(&name) {
                2
            } else {
                1
            };
            continue;
        }
        if token.starts_with('-') && token.len() > 1 {
            index += 1;
            continue;
        }
        break;
    }
    if tokens.get(index).is_none_or(|token| token != "remote") {
        return arguments;
    }
    let first = index + 1;
    let mut cursor = first;
    while cursor < tokens.len() {
        let token = &tokens[cursor];
        if token == "--" {
            return arguments;
        }
        let Some(name) = option_name(token) else {
            if token.starts_with('-') && token.len() > 1 {
                cursor += 1;
                continue;
            }
            break;
        };
        let takes = !token.contains('=')
            && (REMOTE_VALUES.contains(&name) || GLOBAL_VALUES.contains(&name));
        cursor += if takes { 2 } else { 1 };
    }
    if cursor == first || cursor >= tokens.len() {
        return arguments;
    }
    let mut reordered = arguments[..first].to_vec();
    reordered.push(arguments[cursor].clone());
    reordered.extend_from_slice(&arguments[first..cursor]);
    reordered.extend_from_slice(&arguments[cursor + 1..]);
    reordered
}

fn takes_value(command: &clap::Command, name: &str) -> bool {
    if GLOBAL_VALUES.contains(&name) {
        return true;
    }
    command
        .get_arguments()
        .find(|argument| {
            argument.get_long() == Some(name)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
        .is_some_and(|argument| {
            argument.get_action().takes_values()
                && argument
                    .get_num_args()
                    .is_none_or(|range| range.min_values() > 0)
        })
}

fn declared(command: &clap::Command, name: &str) -> bool {
    let root = Cli::command();
    command
        .get_arguments()
        .chain(root.get_arguments())
        .any(|argument| {
            argument.get_long() == Some(name)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
}

fn flag(command: &clap::Command, name: &str) -> bool {
    let root = Cli::command();
    command
        .get_arguments()
        .chain(root.get_arguments())
        .find(|argument| {
            argument.get_long() == Some(name)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
        .is_some_and(|argument| !argument.get_action().takes_values())
}

fn same_flag(command: &clap::Command, token: &str, name: &str) -> bool {
    let root = Cli::command();
    let Some(argument) = command
        .get_arguments()
        .chain(root.get_arguments())
        .find(|argument| {
            argument.get_long() == Some(name)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
    else {
        return false;
    };
    if let Some(long) = token.strip_prefix("--") {
        return argument.get_long() == Some(long)
            || argument
                .get_all_aliases()
                .is_some_and(|aliases| aliases.contains(&long));
    }
    token == format!("-{}", argument.get_short().unwrap_or('\0'))
}

fn short_of(command: &clap::Command, name: &str) -> Option<char> {
    let root = Cli::command();
    command
        .get_arguments()
        .chain(root.get_arguments())
        .find(|argument| argument.get_long() == Some(name))
        .and_then(clap::Arg::get_short)
}

fn consumes_next(argument: &clap::Arg) -> bool {
    argument.get_action().takes_values()
        && !argument.is_require_equals_set()
        && argument
            .get_num_args()
            .is_none_or(|range| range.min_values() > 0)
}

fn wants_value(command: &clap::Command, token: &str) -> bool {
    let root = Cli::command();
    let mut arguments = command.get_arguments().chain(root.get_arguments());
    if let Some(long) = token.strip_prefix("--") {
        return !long.contains('=')
            && arguments.any(|argument| {
                consumes_next(argument)
                    && (argument.get_long() == Some(long)
                        || argument
                            .get_all_aliases()
                            .is_some_and(|aliases| aliases.contains(&long)))
            });
    }
    let Some(letters) = token.strip_prefix('-') else {
        return false;
    };
    let mut characters = letters.chars();
    match (characters.next(), characters.next()) {
        (Some(letter), None) => arguments
            .any(|argument| argument.get_short() == Some(letter) && consumes_next(argument)),
        _ => false,
    }
}

fn forget_flag(output: &mut Vec<OsString>, command: &clap::Command, name: &str) {
    let plain = format!("--{name}");
    let short = short_of(command, name);
    let mut kept: Vec<OsString> = Vec::with_capacity(output.len());
    let mut value_next = false;
    for token in output.drain(..) {
        let text = token.to_string_lossy().into_owned();
        if std::mem::take(&mut value_next) {
            kept.push(token);
            continue;
        }
        value_next = wants_value(command, &text);
        if text == plain || same_flag(command, &text, name) {
            continue;
        }
        let grouped = text
            .strip_prefix('-')
            .filter(|letters| !letters.starts_with('-') && letters.chars().count() > 1)
            .filter(|letters| {
                letters
                    .chars()
                    .all(|letter| short_flag(command, &format!("-{letter}")).is_some())
            });
        match (grouped, short) {
            (Some(letters), Some(short)) if letters.contains(short) => {
                let rest: String = letters.chars().filter(|letter| *letter != short).collect();
                if !rest.is_empty() {
                    kept.push(format!("-{rest}").into());
                }
            }
            _ => kept.push(token),
        }
    }
    *output = kept;
}

fn short_flag(command: &clap::Command, token: &str) -> Option<String> {
    let short = token.strip_prefix('-')?.chars().next()?;
    let root = Cli::command();
    command
        .get_arguments()
        .chain(root.get_arguments())
        .find(|argument| argument.get_short() == Some(short))
        .filter(|argument| !argument.get_action().takes_values())
        .and_then(|argument| argument.get_long().map(str::to_owned))
}

enum Spelling {
    Keep,
    Replace(String),
}

const RENAMED: &[(&str, &str, &str)] = &[
    ("stats", "--watch", "--wait"),
    ("read", "--text", "--query"),
    ("link", "--no-browser", "--no-open"),
];

fn legacy_spelling(text: &str, notices: &mut Vec<(String, String)>) -> Spelling {
    if let Some(single) = text.strip_prefix('-').filter(|rest| !rest.starts_with('-')) {
        let name = single.split_once('=').map_or(single, |(name, _)| name);
        if name.len() > 1 && known(name) {
            notices.push((format!("-{name}"), format!("--{name}")));
            return Spelling::Replace(format!("--{single}"));
        }
    }
    Spelling::Keep
}

#[allow(clippy::too_many_lines)]
pub fn prepare(raw: Vec<OsString>) -> Prepared {
    let mut prepared = Prepared::default();
    let mut iterator = raw.into_iter();
    if let Some(program) = iterator.next() {
        prepared.arguments.push(program);
    }
    let mut after_separator = false;
    let mut rewritten = Vec::new();
    for argument in iterator {
        if after_separator {
            rewritten.push(argument);
            continue;
        }
        let Some(text) = argument.to_str().map(str::to_owned) else {
            rewritten.push(argument);
            continue;
        };
        if text == "--" {
            after_separator = true;
            rewritten.push(argument);
            continue;
        }
        match legacy_spelling(&text, &mut prepared.notices) {
            Spelling::Replace(replacement) => rewritten.push(replacement.into()),
            Spelling::Keep => rewritten.push(argument),
        }
    }
    let rewritten = hoist_remote_options(rewritten);
    let tokens: Vec<String> = rewritten
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let before: Vec<&String> = tokens.iter().take_while(|token| *token != "--").collect();
    prepared.help = before
        .iter()
        .any(|token| *token == "-h" || *token == "--help");
    prepared.version = before
        .iter()
        .any(|token| matches!(token.as_str(), "-v" | "-V" | "--version"));
    let (path, typed, leaf, stray) = resolve(&tokens);
    prepared.path.clone_from(&path);
    prepared.typed = typed;
    prepared.stray = stray;
    for (command, old, new) in RENAMED {
        if path.first().is_some_and(|first| first == command)
            && before
                .iter()
                .any(|token| *token == old || token.starts_with(&format!("{old}=")))
        {
            prepared
                .notices
                .push(((*old).to_owned(), (*new).to_owned()));
        }
    }
    let tolerant = path
        .first()
        .is_some_and(|first| TOLERANT_COMMANDS.contains(&first.as_str()));
    let read_only = READ_ONLY_LEAVES.contains(&path.join(" ").as_str());
    let shown = format!("bridge {}", path.join(" "));
    let mut output = Vec::with_capacity(rewritten.len());
    let mut remaining = rewritten.into_iter();
    let mut after = false;
    let mut value_next = false;
    while let Some(mut argument) = remaining.next() {
        let mut text = argument.to_string_lossy().into_owned();
        if after || text == "--" {
            after = true;
            output.push(argument);
            continue;
        }
        if std::mem::take(&mut value_next) {
            output.push(argument);
            continue;
        }
        value_next = wants_value(&leaf, &text);
        if let Some(long) = text.strip_prefix("--").map(str::to_owned) {
            if let Some((flag_name, value)) = long.split_once('=')
                && matches!(value, "true" | "false")
                && (flag(&leaf, flag_name) || LEGACY_BOOLEANS.contains(&flag_name))
            {
                let plain = format!("--{flag_name}");
                forget_flag(&mut output, &leaf, flag_name);
                if value == "false" {
                    continue;
                }
                text.clone_from(&plain);
                argument = plain.into();
            } else if let Some(negated) = long.strip_prefix("no-")
                && !long.contains('=')
                && !declared(&leaf, &long)
                && flag(&leaf, negated)
            {
                forget_flag(&mut output, &leaf, negated);
                continue;
            } else if !long.contains('=') && flag(&leaf, &long) {
                forget_flag(&mut output, &leaf, &long);
            }
        } else if text.len() == 2
            && text.starts_with('-')
            && let Some(name) = short_flag(&leaf, &text)
        {
            forget_flag(&mut output, &leaf, &name);
        }
        let name = text.strip_prefix("--").map(|long| {
            long.split_once('=')
                .map_or(long, |(name, _)| name)
                .to_owned()
        });
        match name {
            Some(name)
                if tolerant
                    && known(&name)
                    && !declared(&leaf, &name)
                    && (read_only || !CONSEQUENTIAL.contains(&name.as_str())) =>
            {
                prepared.notices.push((
                    format!("ignored:--{name}"),
                    format!(
                        "--{name} is ignored by {} and will be refused in the next major version.",
                        shown.trim_end()
                    ),
                ));
                if LEGACY_VALUES.contains(&name.as_str()) && !text.contains('=') {
                    remaining.next();
                }
            }
            _ => output.push(argument),
        }
    }
    prepared.arguments.extend(output);
    prepared
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arguments: &[&str]) -> (Vec<String>, Vec<String>) {
        let prepared = prepare(arguments.iter().map(OsString::from).collect());
        (
            prepared
                .arguments
                .into_iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            prepared.notices.into_iter().map(|(old, _)| old).collect(),
        )
    }

    #[test]
    fn single_dash_long_options_still_work_and_warn() {
        let (arguments, notices) = run(&["bridge", "status", "-home", "/h"]);
        assert_eq!(arguments, ["bridge", "status", "--home", "/h"]);
        assert_eq!(notices, ["-home"]);
        assert_eq!(
            run(&["bridge", "status", "-home=/h"]).0,
            ["bridge", "status", "--home=/h"]
        );
    }

    #[test]
    fn boolean_values_and_negations_are_accepted_quietly() {
        let (arguments, notices) = run(&["bridge", "pair", "--iphone=true", "--via-remote=false"]);
        assert_eq!(arguments, ["bridge", "pair", "--iphone"]);
        assert!(notices.is_empty(), "{notices:?}");
        let (arguments, notices) = run(&["bridge", "status", "--json=true"]);
        assert_eq!(arguments, ["bridge", "status", "--json"]);
        assert!(notices.is_empty(), "{notices:?}");
        let (arguments, _) = run(&["bridge", "--json", "stats", "--no-json", "--all=false"]);
        assert_eq!(arguments, ["bridge", "stats"]);
        let (arguments, _) = run(&["bridge", "-y", "agents", "revoke", "x", "--yes=false"]);
        assert_eq!(arguments, ["bridge", "agents", "revoke", "x"]);
        let (arguments, _) = run(&["bridge", "link", "claude", "--no-open"]);
        assert_eq!(arguments, ["bridge", "link", "claude", "--no-open"]);
    }

    #[test]
    fn nothing_after_the_separator_is_rewritten() {
        let (arguments, notices) =
            run(&["bridge", "skills", "get", "--", "--force=false", "-home"]);
        assert_eq!(
            arguments,
            ["bridge", "skills", "get", "--", "--force=false", "-home"]
        );
        assert_eq!(notices.len(), 0, "{notices:?}");
    }

    #[test]
    fn reads_drop_unused_legacy_options_with_a_warning_wherever_home_sits() {
        for arguments in [
            &[
                "bridge", "status", "--force", "--name", "zz", "--via", "relay",
            ][..],
            &[
                "bridge", "--home", "/h", "status", "--force", "--name", "zz", "--via", "relay",
            ][..],
            &[
                "bridge",
                "status",
                "--force",
                "--home",
                "/h",
                "--name=zz",
                "--via",
                "relay",
            ][..],
        ] {
            let (kept, notices) = run(arguments);
            assert!(
                !kept.iter().any(|argument| argument.starts_with("--force")
                    || argument.starts_with("--name")
                    || argument.starts_with("--via")),
                "{kept:?}"
            );
            assert_eq!(notices.len(), 3, "{notices:?}");
        }
    }

    #[test]
    fn mutations_never_drop_inputs_that_change_safety_or_target() {
        let (kept, notices) = run(&["bridge", "agent", "revoke", "agent:x", "--force"]);
        assert!(kept.contains(&"--force".to_owned()));
        assert_eq!(notices.len(), 0, "{notices:?}");
        let (kept, _) = run(&["bridge", "pair", "--name", "r", "--force"]);
        assert!(kept.contains(&"--force".to_owned()));
        assert!(!kept.contains(&"--name".to_owned()));
    }

    #[test]
    fn new_commands_stay_strict() {
        let (kept, notices) = run(&["bridge", "up", "--force", "--name", "r"]);
        assert_eq!(kept, ["bridge", "up", "--force", "--name", "r"]);
        assert_eq!(notices.len(), 0, "{notices:?}");
    }

    #[test]
    fn help_and_version_are_found_before_the_separator_only() {
        assert!(
            prepare(
                ["bridge", "read", "--bad", "--help"]
                    .map(OsString::from)
                    .to_vec()
            )
            .help
        );
        assert!(
            !prepare(
                ["bridge", "call", "--", "--help"]
                    .map(OsString::from)
                    .to_vec()
            )
            .help
        );
        assert!(
            prepare(
                ["bridge", "agents", "list", "-v"]
                    .map(OsString::from)
                    .to_vec()
            )
            .version
        );
        assert_eq!(
            prepare(
                ["bridge", "--home", "/h", "agent", "ls"]
                    .map(OsString::from)
                    .to_vec()
            )
            .path,
            ["agents", "list"]
        );
    }
}
