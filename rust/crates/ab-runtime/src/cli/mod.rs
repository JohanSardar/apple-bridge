mod commands;
mod compat;
mod failure;
mod gateway;
mod link;
mod permissions;
mod read;
mod skills;
mod stats;
mod tree;

pub use compat::normalize;
pub use failure::{Failure, Outcome};
pub use tree::{Cli, Command};

use crate::default_home;
use clap::Parser;
use std::{io::IsTerminal, path::PathBuf, process::ExitCode};
use tree::{ConnectorAction, PermissionsAction, SkillsAction};

pub fn parse(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(normalize(arguments))
}

fn report_parse_error(error: &clap::Error) -> ExitCode {
    let code = if error.use_stderr() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };
    if error.print().is_err() {
        return ExitCode::FAILURE;
    }
    code
}

pub async fn main() -> ExitCode {
    let cli = match parse(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => return report_parse_error(&error),
    };
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {}", failure.message);
            if let Some(next) = failure.next {
                eprintln!("next: {next}");
            }
            ExitCode::FAILURE
        }
    }
}

fn resolve_home(given: Option<PathBuf>) -> Outcome<PathBuf> {
    given.map_or_else(
        || {
            default_home().map_err(|_| {
                Failure::new("Could not find your home folder.")
                    .next("Set HOME, or pass --home with the folder to use.")
            })
        },
        Ok,
    )
}

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

async fn run(cli: Cli) -> Outcome {
    let home = resolve_home(cli.home)?;
    match cli.command {
        Command::Up(args) => gateway::up(&home, &args).await,
        Command::Down => gateway::down(&home),
        Command::Link(args) => link::link(&home, &args).await,
        Command::Unlink(args) => link::unlink(&args),
        Command::Stats(args) => stats::run(&home, &args).await,
        Command::Read(args) => read::read(&home, &args).await,
        Command::Search(args) => read::search(&home, &args).await,
        Command::Call(args) => read::call(&home, &args).await,
        Command::Skills(command) => match command.action {
            SkillsAction::List => {
                skills::list();
                Ok(())
            }
            SkillsAction::Get { name } => skills::get(&name),
        },
        Command::Permissions(command) => match command.action {
            PermissionsAction::Guide(args) => permissions::guide(&home, &args).await,
            PermissionsAction::Status(args) => permissions::status(&home, &args),
            PermissionsAction::Request { name, helper_app } => {
                commands::permission_request(&name, helper_app.as_deref())
            }
        },
        Command::Connector(command) => commands::connector(&home, command.action).await,
        Command::Status => commands::status(&home),
        Command::Doctor => commands::doctor(&home),
        Command::Remote(args) => commands::remote(&home, &args).await,
        Command::Agent(command) => commands::agent(&home, command.action),
        Command::Start(args) => commands::start(&home, args).await,
        Command::Pair(args) => commands::pair(&home, &args),
        Command::Join(args) => commands::join(&home, &args).await,
        Command::Connect(args) => commands::connect(&args).await,
        Command::Token(command) => commands::token(&home, command.action),
        Command::Revoke(args) => commands::revoke(&home, &args.id),
        Command::Scope(args) => commands::scope(&home, &args),
        Command::Web(command) => commands::web(&home, command.action),
        Command::Backup(args) => commands::backup(&home, &args.path),
        Command::Restore(args) => commands::restore(&home, args),
        Command::InstallCredentials => commands::install_credentials(&home),
        Command::Shortcuts(command) => commands::shortcuts(command.action).await,
        Command::Selftest(command) => {
            let tree::SelftestAction::Messages(args) = command.action;
            commands::selftest(&args)
        }
        Command::Setup(args) => commands::selftest(&args),
        #[cfg(feature = "lab-harness")]
        Command::LabOperationCatalog => commands::lab_operation_catalog(),
        #[cfg(feature = "lab-harness")]
        Command::LabCredentialLifecycle => commands::lab_credential_lifecycle(&home),
        #[cfg(feature = "lab-harness")]
        Command::LabSession => commands::lab_session(&home),
        #[cfg(feature = "lab-harness")]
        Command::LabPrincipal(args) => commands::lab_principal(&home, &args),
    }
}

fn connector_failure(home: &std::path::Path, action: &ConnectorAction) -> Failure {
    if home.join("run/connectors.sock").exists() {
        match action {
            ConnectorAction::Pending => {
                Failure::new("The gateway did not answer.").next("Run `bridge up`, then try again.")
            }
            _ => Failure::new("That request was refused: the code is wrong, or it has expired.")
                .next("Run `bridge connector pending` and check the code on the app's page."),
        }
    } else {
        Failure::not_running(home)
    }
}
