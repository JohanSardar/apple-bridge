use std::ffi::{OsStr, OsString};

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

pub fn normalize(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut output = Vec::new();
    for argument in arguments {
        match rewrite(&argument) {
            Rewrite::Keep => output.push(argument),
            Rewrite::Drop => {}
            Rewrite::Replace(replacement) => output.push(replacement),
        }
    }
    output
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
