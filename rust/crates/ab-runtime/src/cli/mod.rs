mod access;
mod argv;
mod children;
mod commands;
mod connect;
mod failure;
mod gateway;
mod help;
mod link;
mod modes;
mod permissions;
mod read;
mod skills;
mod stats;
mod tree;
mod ui;

pub use failure::{Failure, Outcome};
pub use tree::{Cli, Command};

use crate::default_home;
use clap::{
    Parser,
    error::{ContextKind, ContextValue, ErrorKind},
};
use failure::{Reason, Step};
use modes::{Modes, Requested};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tree::{
    AgentAction, CertificatesAction, ConnectorAction, PermissionsAction, RemoteAction,
    SelftestAction, SessionsAction, ShortcutsAction, SkillsAction, TokenAction, WebAction,
};

#[cfg(target_os = "macos")]
const SOCKET_LIMIT: usize = 103;
#[cfg(not(target_os = "macos"))]
const SOCKET_LIMIT: usize = 107;
const LONGEST_SOCKET: &str = "run/connectors.sock";

static OWNS_INTERRUPT: AtomicBool = AtomicBool::new(false);

pub fn own_interrupt() {
    OWNS_INTERRUPT.store(true, Ordering::SeqCst);
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(argv::prepare(arguments.into_iter().collect()).arguments)
}

fn requested(matches: &clap::ArgMatches) -> Requested {
    use clap::{FromArgMatches, parser::ValueSource};
    let globals = tree::Globals::from_arg_matches(deepest(matches)).unwrap_or(tree::Globals {
        home: None,
        cwd: None,
        json: false,
        fields: None,
        non_interactive: None,
        yes: false,
        no_color: false,
        accessible: false,
        debug: false,
        version: false,
        help: false,
    });
    let explicit = deepest(matches)
        .value_source("home")
        .is_some_and(|source| source == ValueSource::CommandLine);
    Requested {
        json: globals.json,
        non_interactive: globals.non_interactive,
        yes: globals.yes,
        no_color: globals.no_color,
        debug: globals.debug,
        explicit_home: globals.home.filter(|_| explicit),
        explicit_cwd: globals.cwd,
        fields: globals.fields.as_deref().map(split_fields),
        accessible: globals.accessible,
    }
}

fn scanned(arguments: &[OsString]) -> Requested {
    let tokens: Vec<String> = arguments
        .iter()
        .skip(1)
        .map(|argument| argument.to_string_lossy().into_owned())
        .take_while(|token| token != "--")
        .collect();
    let value = |name: &str| {
        let long = format!("--{name}");
        let joined = format!("--{name}=");
        let mut found = None;
        for (index, token) in tokens.iter().enumerate() {
            if *token == long {
                found = tokens.get(index + 1).cloned();
            } else if let Some(rest) = token.strip_prefix(&joined) {
                found = Some(rest.to_owned());
            }
        }
        found
    };
    let present = |names: &[&str]| tokens.iter().any(|token| names.contains(&token.as_str()));
    Requested {
        json: present(&["--json"]),
        non_interactive: present(&["--non-interactive"]).then_some(true),
        yes: present(&["--yes", "-y"]),
        no_color: present(&["--no-color"]),
        debug: present(&["--debug", "-d"]),
        explicit_home: value("home").map(PathBuf::from),
        explicit_cwd: value("cwd").map(PathBuf::from),
        fields: value("fields").as_deref().map(split_fields),
        accessible: present(&["--accessible"]),
    }
}

fn deepest(matches: &clap::ArgMatches) -> &clap::ArgMatches {
    let mut current = matches;
    while let Some((_, child)) = current.subcommand() {
        current = child;
    }
    current
}

fn empty_home_variable() -> bool {
    std::env::var_os("APPLE_BRIDGE_HOME").is_some_and(|value| value.is_empty())
}

fn cli_command() -> clap::Command {
    use clap::CommandFactory;
    let command = Cli::command();
    if empty_home_variable() {
        command.mut_arg("home", |argument| argument.env(None::<&str>))
    } else {
        command
    }
}

fn lenient_matches(arguments: &[OsString]) -> clap::ArgMatches {
    cli_command()
        .ignore_errors(true)
        .try_get_matches_from(arguments)
        .unwrap_or_default()
}

fn split_fields(fields: &str) -> Vec<String> {
    fields
        .split(',')
        .map(|field| field.trim().to_owned())
        .filter(|field| !field.is_empty())
        .collect()
}

fn context(error: &clap::Error, kind: ContextKind) -> Option<String> {
    match error.get(kind)? {
        ContextValue::String(text) => Some(text.clone()),
        ContextValue::Strings(texts) => texts.first().cloned(),
        ContextValue::StyledStr(text) => Some(text.to_string()),
        ContextValue::StyledStrs(texts) => texts.first().map(ToString::to_string),
        _ => None,
    }
}

fn shown_path(path: &[String]) -> String {
    format!("bridge {}", path.join(" ")).trim_end().to_owned()
}

fn help_step(path: &[String]) -> Step {
    let mut argv = vec!["bridge".to_owned()];
    argv.extend(path.iter().cloned());
    argv.push("--help".to_owned());
    Step::new("See all options", argv)
}

fn unknown_command(path: &[String], typed: &str) -> Failure {
    let mut headline = format!(
        "Unknown command {} for {}.",
        ui::clean_input(typed),
        shown_path(path)
    );
    if let Some(close) = help::suggestion(&help::child_names(path), typed) {
        headline = format!("{headline} Did you mean {close}?");
    }
    let mut step = help_step(path);
    "See the commands".clone_into(&mut step.benefit);
    Failure::new(Reason::UnknownCommand, headline).step(step)
}

fn parse_failure(error: &clap::Error, path: &[String]) -> Failure {
    let shown = shown_path(path);
    match error.kind() {
        ErrorKind::InvalidSubcommand => {
            let typed = context(error, ContextKind::InvalidSubcommand).unwrap_or_default();
            unknown_command(path, &typed)
        }
        ErrorKind::UnknownArgument => {
            let typed = context(error, ContextKind::InvalidArg).unwrap_or_default();
            if typed.starts_with('-') {
                let name = typed
                    .split_once('=')
                    .map_or(typed.as_str(), |(name, _)| name)
                    .to_owned();
                let mut headline = format!("Unknown option {} for {shown}.", ui::clean(&name));
                if let Some(close) = help::suggestion(&help::option_names(path), &name) {
                    headline = format!("{headline} Did you mean {close}?");
                }
                Failure::new(Reason::UnknownOption, headline).step(help_step(path))
            } else if help::find(path).is_some_and(|command| command.has_subcommands()) {
                unknown_command(path, &typed)
            } else {
                Failure::new(
                    Reason::InvalidArguments,
                    format!(
                        "Unexpected argument {} for {shown}.",
                        ui::clean_input(&typed)
                    ),
                )
                .step(help_step(path))
            }
        }
        ErrorKind::InvalidValue => {
            let argument = context(error, ContextKind::InvalidArg).unwrap_or_default();
            let value = context(error, ContextKind::InvalidValue).unwrap_or_default();
            let accepted = match error.get(ContextKind::ValidValue) {
                Some(ContextValue::Strings(values)) => values.join(", "),
                _ => String::new(),
            };
            let argument = argument.split(' ').next().unwrap_or_default().to_owned();
            let mut headline = if value.is_empty() {
                format!("Can't use {argument} without a value.")
            } else {
                format!("Can't use {} for {argument}.", ui::clean_input(&value))
            };
            if !accepted.is_empty() {
                headline = format!("{headline} Accepted: {accepted}.");
            }
            Failure::new(Reason::InvalidValue, headline).step(help_step(path))
        }
        ErrorKind::MissingRequiredArgument => {
            let missing = context(error, ContextKind::InvalidArg).unwrap_or_default();
            Failure::new(
                Reason::MissingArgument,
                format!("Can't run {shown} without {}.", ui::clean(&missing)),
            )
            .step(help_step(path))
        }
        ErrorKind::ArgumentConflict => {
            let first = context(error, ContextKind::InvalidArg).unwrap_or_default();
            let second = context(error, ContextKind::PriorArg).unwrap_or_default();
            if first == second {
                let name = first.split(' ').next().unwrap_or_default().to_owned();
                return Failure::new(
                    Reason::InvalidArguments,
                    format!("{name} can be given once."),
                )
                .step(help_step(path));
            }
            Failure::new(
                Reason::ConflictingOptions,
                format!("Can't use {first} with {second}."),
            )
            .step(help_step(path))
        }
        _ => {
            let text = error.to_string();
            let first = text
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches("error: ")
                .to_owned();
            Failure::new(
                Reason::InvalidArguments,
                format!("Can't run {shown}: {first}."),
            )
            .step(help_step(path))
        }
    }
}

fn print_help(path: &[String]) -> ExitCode {
    if Modes::get().json {
        ui::json_document(help::document(path));
    } else {
        ui::out(&help::render(path));
    }
    ExitCode::SUCCESS
}

fn finish(result: Outcome) -> ExitCode {
    match result {
        Ok(()) => {
            ui::report_legacy_warnings();
            ExitCode::SUCCESS
        }
        Err(failure) => {
            ui::render_failure(&failure);
            ExitCode::from(failure.exit_code())
        }
    }
}

fn group_without_action(command: &Command) -> Option<&'static str> {
    Some(match command {
        Command::Skills(group) if group.action.is_none() => "skills",
        Command::Permissions(group) if group.action.is_none() => "permissions",
        Command::Agents(group) if group.action.is_none() => "agents",
        Command::Tokens(group) if group.action.is_none() => "tokens",
        Command::Connectors(group) if group.action.is_none() => "connectors",
        Command::Web(group) if group.action.is_none() => "web",
        Command::Remote(group) if group.action.is_none() => "remote",
        Command::Selftest(group) if group.action.is_none() => "selftest",
        Command::Shortcuts(group) if group.action.is_none() => "shortcuts",
        Command::Web(group) => match &group.action {
            Some(WebAction::Sessions(sessions)) if sessions.action.is_none() => "web sessions",
            _ => return None,
        },
        Command::Remote(group) => match &group.action {
            Some(RemoteAction::Certificates(certificates)) if certificates.action.is_none() => {
                "remote certificates"
            }
            _ => return None,
        },
        _ => return None,
    })
}

pub async fn main() -> ExitCode {
    use clap::FromArgMatches;
    let raw: Vec<OsString> = std::env::args_os().collect();
    let prepared = argv::prepare(raw);
    access::typed_as(&prepared.typed);
    modes::set_typed(&prepared.typed);
    let parsed = cli_command().try_get_matches_from(&prepared.arguments);
    let requested = if let Ok(matches) = &parsed {
        requested(matches)
    } else {
        let mut scan = scanned(&prepared.arguments);
        let lenient = requested(&lenient_matches(&prepared.arguments));
        scan.explicit_home = scan.explicit_home.or(lenient.explicit_home);
        scan
    };
    let cwd = requested.explicit_cwd.clone();
    Modes::resolve(requested).install();
    for (old, replacement) in &prepared.notices {
        if old.starts_with("ignored:") {
            ui::warn("deprecated", replacement);
        } else {
            ui::deprecated(old, replacement);
        }
    }
    if prepared.help {
        if let Some(stray) = &prepared.stray {
            return finish(Err(unknown_command(&prepared.path, stray)));
        }
        return print_help(&prepared.path);
    }
    if prepared.version {
        if Modes::get().json {
            ui::json_document(serde_json::json!({"version": env!("CARGO_PKG_VERSION")}));
        } else {
            ui::outln(&format!("bridge {}", env!("CARGO_PKG_VERSION")));
        }
        return ExitCode::SUCCESS;
    }
    let cli = match parsed.and_then(|matches| Cli::from_arg_matches(&matches)) {
        Ok(cli) => cli,
        Err(error) => return finish(Err(parse_failure(&error, &prepared.path))),
    };
    let Some(command) = cli.command else {
        return print_help(&[]);
    };
    if let Some(cwd) = &cwd
        && !matches!(command, Command::Help(_) | Command::Completion(_))
        && let Err(failure) = enter(cwd)
    {
        return finish(Err(failure));
    }
    if let Some(group) = group_without_action(&command) {
        let path: Vec<String> = group.split(' ').map(str::to_owned).collect();
        return print_help(&path);
    }
    if let Some(replacement) = help::deprecated(&prepared.path.join(" ")) {
        ui::deprecated(&shown_path(&prepared.path), replacement);
    }
    if empty_home_variable() && cli.globals.home.is_none() {
        return finish(Err(Failure::new(
            Reason::InvalidHome,
            "Can't use an empty APPLE_BRIDGE_HOME. It must name a folder.",
        )
        .sentence("Set it to a folder, or unset it to use the default.")
        .then(
            "Or name the folder for this run",
            ["bridge", "status", "--home", "<DIR>"],
        )));
    }
    let home = match resolve_home(cli.globals.home) {
        Ok(home) => home,
        Err(failure) => return finish(Err(failure)),
    };
    if let Err(failure) = check_fields(&prepared.path) {
        return finish(Err(failure));
    }
    supervise(command, home).await
}

fn enter(cwd: &Path) -> Outcome {
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --cwd {}. It isn't a folder.",
                ui::shell_quote(&cwd.to_string_lossy())
            ),
        ));
    }
    std::env::set_current_dir(cwd).map_err(|error| {
        Failure::new(
            Reason::PermissionDenied,
            format!(
                "Can't use --cwd {}.",
                ui::shell_quote(&cwd.to_string_lossy())
            ),
        )
        .row("Detail", ui::clean(&error.to_string()))
    })
}

fn check_fields(path: &[String]) -> Outcome {
    let Some(fields) = &Modes::get().fields else {
        return Ok(());
    };
    let key = path.join(" ");
    if matches!(key.as_str(), "read" | "search") {
        return Ok(());
    }
    let schema = help::leaf(&key).map(|leaf| leaf.schema).unwrap_or_default();
    if schema.is_empty() {
        return Err(Failure::usage(format!(
            "Can't use --fields with {}. It has no result schema.",
            shown_path(path)
        )));
    }
    let unknown: Vec<&String> = fields
        .iter()
        .filter(|field| !schema.iter().any(|(name, _, _)| name == field))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let valid: Vec<&str> = schema.iter().map(|(name, _, _)| *name).collect();
    Err(Failure::new(
        Reason::InvalidValue,
        format!(
            "Can't use --fields {}. Valid fields: {}.",
            unknown
                .iter()
                .map(|field| ui::clean(field))
                .collect::<Vec<_>>()
                .join(","),
            valid.join(", ")
        ),
    ))
}

async fn supervise(command: Command, home: PathBuf) -> ExitCode {
    use tokio::signal::unix::{SignalKind, signal};
    let server = matches!(command, Command::Start(_) | Command::Connect(_));
    let (Ok(mut interrupt), Ok(mut terminate), Ok(mut hangup)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
    ) else {
        return finish(run(command, home).await);
    };
    let mut task = tokio::spawn(run(command, home));
    let code = tokio::select! {
        joined = &mut task => return finish(joined.unwrap_or_else(|error| Err(Failure::internal(&error)))),
        _ = interrupt.recv() => 130,
        _ = terminate.recv() => 143,
        _ = hangup.recv() => 129,
    };
    if server {
        let _ = tokio::time::timeout(Duration::from_secs(15), &mut task).await;
        children::stop_all();
        std::process::exit(code);
    }
    if code == 130
        && OWNS_INTERRUPT.load(Ordering::SeqCst)
        && let Ok(joined) = tokio::time::timeout(Duration::from_secs(10), &mut task).await
    {
        return finish(joined.unwrap_or_else(|error| Err(Failure::internal(&error))));
    }
    children::stop_all();
    let note = ui::cancel_note();
    let with_step = |failure: Failure| match ui::cancel_step() {
        Some(step) => failure.step(step),
        None => failure,
    };
    if Modes::get().json {
        let failure = with_step(Failure::canceled(
            note.unwrap_or_else(|| "Canceled.".to_owned()),
        ));
        ui::render_failure(&failure.data("signal", code - 128));
        std::process::exit(code);
    }
    if let Some(note) = note {
        ui::render_failure(&with_step(Failure::canceled(format!("\n{note}"))));
    }
    std::process::exit(code);
}

fn resolve_home(given: Option<PathBuf>) -> Outcome<PathBuf> {
    match given {
        Some(home) if home.as_os_str().is_empty() => Err(Failure::new(
            Reason::InvalidHome,
            "Can't use an empty home. --home or APPLE_BRIDGE_HOME must name a folder.",
        )),
        Some(home) => {
            if home.exists() && !home.is_dir() {
                return Err(Failure::new(
                    Reason::InvalidHome,
                    format!(
                        "Can't use {} as a home. It's a file, not a folder.",
                        home.display()
                    ),
                ));
            }
            Ok(home)
        }
        None => default_home().map_err(|_| {
            Failure::new(
                Reason::InvalidHome,
                "Can't find your data folder. HOME isn't set.",
            )
            .then(
                "Name the folder to use",
                ["bridge", "up", "--home", "<DIR>"],
            )
        }),
    }
}

pub(crate) fn check_socket_length(home: &Path) -> Outcome {
    let absolute = if home.is_absolute() {
        home.to_owned()
    } else {
        std::env::current_dir().map_or_else(|_| home.to_owned(), |cwd| cwd.join(home))
    };
    let length = absolute.join(LONGEST_SOCKET).as_os_str().len();
    if length > SOCKET_LIMIT {
        let platform = if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "this system"
        };
        return Err(Failure::new(
            Reason::InvalidHome,
            format!(
                "Can't use --home {}. Its socket path is {length} characters; {platform} allows {SOCKET_LIMIT}.",
                ui::shell_quote(&home.to_string_lossy())
            ),
        )
        .sentence("Choose a shorter folder, such as one directly under your home folder."));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn run(command: Command, home: PathBuf) -> Outcome {
    let home = home.as_path();
    match command {
        Command::Up(args) => gateway::up(home, &args).await,
        Command::Down => gateway::down(home),
        Command::Link(args) if args.client == tree::LinkClient::ClaudeApp => {
            connect::claude_app(home).await
        }
        Command::Link(args) => link::link(home, &args).await,
        Command::Tailscale(args) => connect::tailscale(home, &args).await,
        Command::Unlink(args) => link::unlink(&args),
        Command::Stats(args) => stats::run(home, &args).await,
        Command::Read(args) => read::read(home, &args).await,
        Command::Search(args) => read::search(home, &args).await,
        Command::Call(args) => read::call(home, &args).await,
        Command::Skills(group) => match group.action {
            Some(SkillsAction::List) => skills::list(),
            Some(SkillsAction::Get { name }) => skills::get(&name),
            None => Ok(()),
        },
        Command::Permissions(group) => match group.action {
            Some(PermissionsAction::Guide(args)) => permissions::guide(home, &args).await,
            Some(PermissionsAction::Status(args)) => permissions::status(home, &args),
            Some(PermissionsAction::Request { name, helper_app }) => {
                commands::permission_request(home, &name, helper_app.as_deref()).await
            }
            None => Ok(()),
        },
        Command::Agents(group) => match group.action {
            Some(AgentAction::List(args)) => access::agents_list(home, &args),
            Some(AgentAction::Create(args)) => access::agents_create(home, &args),
            Some(AgentAction::Inspect(args)) => access::agents_inspect(home, &args.id),
            Some(AgentAction::Update(args)) => {
                access::agents_update(home, &args.id, args.level.as_deref(), args.scope.as_deref())
            }
            Some(AgentAction::Revoke(args)) => {
                access::revoke(home, &args.id, access::Selector::Agent)
            }
            Some(AgentAction::Level(args)) => access::agents_level(home, &args.id, &args.level),
            None => Ok(()),
        },
        Command::Tokens(group) => match group.action {
            Some(TokenAction::Create {
                scope,
                remote,
                expires_in,
            }) => access::tokens_create(home, &scope, remote, expires_in.as_deref()),
            Some(TokenAction::Revoke(args)) => {
                access::revoke(home, &args.id, access::Selector::Any)
            }
            Some(TokenAction::Update { id, scope }) => access::update_grants(home, &id, &scope),
            None => Ok(()),
        },
        Command::Revoke(args) => access::revoke(home, &args.id, access::Selector::Any),
        Command::Scope(args) => access::update_grants(home, &args.id, &args.scope),
        Command::Connectors(group) => match group.action {
            Some(ConnectorAction::List(args)) => access::connectors_list(home, &args).await,
            Some(ConnectorAction::Pending) => {
                access::connectors_list(home, &tree::ListArgs::default()).await
            }
            Some(ConnectorAction::Allow(decision)) => {
                access::connectors_decide(home, &decision, true).await
            }
            Some(ConnectorAction::Deny(decision)) => {
                access::connectors_decide(home, &decision, false).await
            }
            None => Ok(()),
        },
        Command::Web(group) => match group.action {
            Some(WebAction::Enroll { host, public }) => {
                access::web_enroll(home, host.as_deref(), public)
            }
            Some(WebAction::Sessions(sessions)) => match sessions.action {
                Some(SessionsAction::List(args)) => access::sessions_list(home, &args),
                Some(SessionsAction::Revoke { .. }) => access::sessions_revoke(home),
                None => Ok(()),
            },
            Some(WebAction::Lock) => access::sessions_revoke(home),
            Some(WebAction::Devices) => access::sessions_list(home, &tree::ListArgs::default()),
            None => Ok(()),
        },
        Command::Remote(group) => match group.action {
            Some(RemoteAction::Enable(args)) => {
                commands::remote_enable(home, &args, commands::MODERN).await
            }
            Some(RemoteAction::On(args)) => {
                commands::remote_enable(home, &args, commands::LEGACY).await
            }
            Some(RemoteAction::Disable(args)) => {
                commands::remote_disable(home, &args, commands::MODERN).await
            }
            Some(RemoteAction::Off(args)) => {
                commands::remote_disable(home, &args, commands::LEGACY).await
            }
            Some(RemoteAction::Status(args)) => commands::remote_status(home, &args),
            Some(RemoteAction::Key(args)) => commands::remote_key(home, &args),
            Some(RemoteAction::Certificates(certificates)) => match certificates.action {
                Some(CertificatesAction::Accept(args)) => {
                    commands::certificate_accept(home, args.fingerprint.as_deref())
                }
                None => Ok(()),
            },
            Some(RemoteAction::CertificateAccept(args)) => {
                commands::certificate_accept(home, args.fingerprint.as_deref())
            }
            None => Ok(()),
        },
        Command::Pair(args) => commands::pair(home, &args),
        Command::Join(args) => commands::join(home, &args).await,
        Command::Connect(args) => commands::connect(&args).await,
        Command::Start(args) => commands::start(home, args).await,
        Command::Status => commands::status(home),
        Command::Doctor(args) => commands::doctor(home, &args),
        Command::Backup(args) => commands::backup(home, &args),
        Command::Restore(args) => commands::restore(home, args),
        Command::InstallCredentials => commands::install_credentials(home),
        Command::Shortcuts(group) => match group.action {
            Some(ShortcutsAction::Install(args) | ShortcutsAction::InstallPack(args)) => {
                commands::shortcuts(args.pack).await
            }
            None => Ok(()),
        },
        Command::Selftest(group) => match group.action {
            Some(SelftestAction::Messages(args)) => commands::selftest(&args),
            None => Ok(()),
        },
        Command::Setup(args) => commands::selftest(&args),
        Command::Completion(args) => {
            ui::out(&help::completion(args.shell));
            Ok(())
        }
        Command::Help(args) => help_command(&args),
        #[cfg(feature = "lab-harness")]
        Command::LabOperationCatalog => commands::lab_operation_catalog(),
        #[cfg(feature = "lab-harness")]
        Command::LabCredentialLifecycle => commands::lab_credential_lifecycle(home),
        #[cfg(feature = "lab-harness")]
        Command::LabSession => commands::lab_session(home),
        #[cfg(feature = "lab-harness")]
        Command::LabPrincipal(args) => commands::lab_principal(home, &args),
    }
}

fn help_command(args: &tree::HelpArgs) -> Outcome {
    if args.all {
        if Modes::get().json {
            ui::json_document(help::all_document());
        } else {
            for path in help::every_path() {
                if path.is_empty() || help::find(&path).is_some_and(|command| command.is_hide_set())
                {
                    continue;
                }
                let summary = help::find(&path)
                    .and_then(|command| command.get_about().map(ToString::to_string))
                    .unwrap_or_default();
                ui::outln(&format!("bridge {}\t{summary}", path.join(" ")));
            }
        }
        return Ok(());
    }
    if args.path.first().is_some_and(|first| first == "search") {
        let query = args.path[1..].join(" ");
        if query.trim().is_empty() {
            return Err(
                Failure::new(Reason::MissingArgument, "Can't search without <TEXT>.")
                    .then("Search for a word", ["bridge", "help", "search", "<TEXT>"]),
            );
        }
        let matches = help::search(&query);
        if Modes::get().json {
            ui::json_document(serde_json::json!({
                "commands": matches.iter().map(|(path, summary)| serde_json::json!({"path": path, "summary": summary})).collect::<Vec<_>>()
            }));
        } else if matches.is_empty() {
            ui::outln(&format!("No commands match \"{}\".", ui::clean(&query)));
            ui::next_steps(&[Step::new("See every command", ["bridge", "help", "--all"])]);
        } else {
            let rows: Vec<Vec<String>> = matches
                .into_iter()
                .map(|(path, summary)| vec![path, summary])
                .collect();
            ui::table(&["Command", "Summary"], &rows);
        }
        return Ok(());
    }
    let (path, _) = argv::resolve_path(&args.path);
    if path.len() != args.path.len() {
        return Err(unknown_command(&path, &args.path[path.len()]));
    }
    print_help(&path);
    Ok(())
}
