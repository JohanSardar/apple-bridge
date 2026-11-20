use super::Cli;
use clap::CommandFactory;

const GLOBAL_VALUES: &[&str] = &["home", "fields", "cwd"];

pub fn resolve(tokens: &[String]) -> (Vec<String>, Vec<String>) {
    let mut command = Cli::command();
    let mut path = Vec::new();
    let mut typed = Vec::new();
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
            break;
        };
        path.push(child.get_name().to_owned());
        typed.push(token.clone());
        command = child;
    }
    (path, typed)
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
