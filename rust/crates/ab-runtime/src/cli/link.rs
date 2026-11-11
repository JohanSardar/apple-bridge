use super::{
    Failure, Outcome, children, commands,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    tree::{LinkArgs, LinkClient, LinkScope, UnlinkArgs},
    ui,
};
use std::{
    collections::BTreeSet,
    os::fd::AsFd,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const REGISTER_BUDGET: Duration = Duration::from_secs(20);
const SIGN_IN_WAIT: Duration = Duration::from_secs(600);

struct Plan {
    program: &'static str,
    title: &'static str,
    install: &'static str,
    register: Vec<String>,
    login: Vec<String>,
    register_signs_in: bool,
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn plan(
    client: LinkClient,
    name: &str,
    scope: LinkScope,
    url: &str,
    no_open: bool,
) -> Option<Plan> {
    match client {
        LinkClient::Claude => {
            let mut login = strings(&["mcp", "login", name]);
            if no_open {
                login.push("--no-browser".to_owned());
            }
            Some(Plan {
                program: "claude",
                title: "Claude Code",
                install: "https://code.claude.com",
                register: strings(&[
                    "mcp",
                    "add",
                    "--transport",
                    "http",
                    "--scope",
                    scope.as_str(),
                    name,
                    url,
                ]),
                login,
                register_signs_in: false,
            })
        }
        LinkClient::Codex => Some(Plan {
            program: "codex",
            title: "Codex",
            install: "https://developers.openai.com/codex",
            register: strings(&["mcp", "add", name, "--url", url]),
            login: strings(&["mcp", "login", name]),
            register_signs_in: true,
        }),
        LinkClient::Other | LinkClient::ClaudeApp => None,
    }
}

fn shown(program: &str, arguments: &[String]) -> String {
    let quoted: Vec<String> = arguments
        .iter()
        .map(|argument| ui::shell_quote(argument))
        .collect();
    format!("{program} {}", quoted.join(" "))
}

fn piped_result(text: &str) {
    if !Modes::get().json && !Modes::get().stdout_tty() {
        ui::outln(text);
    }
}

fn print_other(url: &str) {
    if Modes::get().json {
        ui::json_document(serde_json::json!({"mcp": url, "rest": url.trim_end_matches("/mcp")}));
        return;
    }
    ui::outln(url);
    ui::note(
        "Any MCP app on this Mac can use this address. Apps that support sign-in ask you to approve them the first time.",
    );
    ui::note(&format!(
        "Bots without sign-in send a token as Authorization: Bearer <token>. REST uses the same token: POST {}/v1/op/sources.status",
        url.trim_end_matches("/mcp")
    ));
    ui::next_steps(&[Step::new(
        "Create a token with the least access it needs",
        [
            "bridge",
            "agents",
            "create",
            "--name",
            "my-bot",
            "--level",
            "read",
            "--scope",
            "messages:read,contacts:read",
        ],
    )]);
}

#[derive(Debug, Clone, Copy)]
struct Deadline {
    end: Option<Instant>,
    given: Option<Duration>,
}

impl Deadline {
    fn new(given: Option<Duration>) -> Self {
        Self {
            end: given.map(|given| Instant::now() + given),
            given,
        }
    }

    fn within(&self, phase: Duration) -> Instant {
        let phase_end = Instant::now() + phase;
        self.end.map_or(phase_end, |end| end.min(phase_end))
    }
}

struct Phase<'a> {
    what: &'a str,
    confirmed: String,
    check: Option<Step>,
}

fn spawn(plan: &Plan, arguments: &[String], quiet: bool) -> Outcome<std::process::Child> {
    let mut command = Command::new(plan.program);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.args(arguments).stdin(Stdio::null());
    if quiet {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing(plan)
        } else {
            Failure::new(
                Reason::SpawnFailed,
                format!("Couldn't run {}.", plan.program),
            )
            .row("Detail", ui::clean(&error.to_string()))
        }
    })
}

fn drain(
    source: Option<impl std::io::Read + Send + 'static>,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut source) = source {
            let _ = source.read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    });
    receiver
}

fn stop_group(leader: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{leader}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn collect_output(
    receiver: &std::sync::mpsc::Receiver<Vec<u8>>,
    until: Instant,
    leader: u32,
) -> Vec<u8> {
    let grace = until
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(500));
    receiver.recv_timeout(grace).unwrap_or_else(|_| {
        stop_group(leader);
        receiver
            .recv_timeout(Duration::from_millis(200))
            .unwrap_or_default()
    })
}

fn stopped(plan: &Plan, arguments: &[String], phase: &Phase<'_>, deadline: &Deadline) -> Failure {
    let shown_command = shown(plan.program, arguments);
    let headline = match deadline.given {
        Some(given) => format!(
            "Stopped {} after {}. --timeout passed.",
            phase.what,
            commands::friendly(given)
        ),
        None => format!(
            "Stopped {}. {} didn't answer in time.",
            phase.what, plan.title
        ),
    };
    let mut failure = Failure::new(Reason::Timeout, headline)
        .row("Command", &shown_command)
        .row("Confirmed", &phase.confirmed)
        .data("command", shown_command)
        .data("confirmed", phase.confirmed.clone())
        .sentence(format!(
            "Apple Bridge stopped {} and waited for it to exit.",
            plan.program
        ));
    if let Some(check) = &phase.check {
        failure = failure.step(check.clone());
    }
    failure.then(
        "Or try again with more time",
        ["bridge", "link", plan.program, "--timeout", "2m"],
    )
}

const PHASE_BUDGET: Duration = Duration::from_secs(30);
const SLOW_AFTER: Duration = Duration::from_secs(3);

fn run_bounded(
    plan: &Plan,
    arguments: &[String],
    deadline: &Deadline,
    phase: &Phase<'_>,
) -> Outcome<(bool, String)> {
    let mut child = spawn(plan, arguments, true)?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut child = children::Owned::new(child);
    let leader = child.0.id();
    let began = Instant::now();
    let until = deadline.within(PHASE_BUDGET);
    let mut announced = false;
    if Modes::get().accessible {
        ui::note(&format!("Now {}…", phase.what));
    }
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if Instant::now() >= until {
            child.stop();
            stop_group(leader);
            return Err(stopped(plan, arguments, phase, deadline));
        }
        if !announced && began.elapsed() >= SLOW_AFTER {
            announced = true;
            ui::note(&format!("Still waiting for {}…", phase.what));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut text = String::from_utf8_lossy(&collect_output(&stdout, until, leader)).into_owned();
    text.push_str(&String::from_utf8_lossy(&collect_output(
        &stderr, until, leader,
    )));
    Ok((status.success(), text))
}

fn check_step(plan: &Plan, name: &str) -> Step {
    Step::new("Check what was saved", [plan.program, "mcp", "get", name])
}

fn inspect(plan: &Plan, name: &str, deadline: &Deadline, confirmed: &str) -> Outcome<String> {
    run_bounded(
        plan,
        &strings(&["mcp", "get", name]),
        deadline,
        &Phase {
            what: &format!("checking {name} in {}", plan.title),
            confirmed: confirmed.to_owned(),
            check: Some(check_step(plan, name)),
        },
    )
    .map(|(_, text)| text)
}

fn missing(plan: &Plan) -> Failure {
    Failure::new(
        Reason::MissingCommand,
        format!(
            "Can't find {}. {} isn't on your PATH.",
            plan.title, plan.program
        ),
    )
    .sentence(format!(
        "Install it from {}, then run this again.",
        plan.install
    ))
    .then("Or connect another app", ["bridge", "link", "other"])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Added {
    Added,
    Already,
}

fn register(plan: &Plan, name: &str, url: &str, deadline: &Deadline) -> Outcome<Added> {
    let (success, text) = run_bounded(
        plan,
        &plan.register,
        deadline,
        &Phase {
            what: &format!("adding {name} to {}", plan.title),
            confirmed: format!("not known whether {} saved {name}", plan.title),
            check: Some(check_step(plan, name)),
        },
    )?;
    if success {
        ui::done("Added", &format!("{name} to {} at {url}", plan.title));
        return Ok(Added::Added);
    }
    if text.contains("already exists") {
        let details = inspect(
            plan,
            name,
            deadline,
            &format!("{} already has a server named {name}", plan.title),
        )?;
        if details.contains(url) {
            ui::receipt(
                "Already added",
                &format!("{name} to {} at {url}", plan.title),
                &format!("{name} is already in {} at {url}.", plan.title),
            );
            return Ok(Added::Already);
        }
        return Err(Failure::new(
            Reason::Conflict,
            format!(
                "Can't add {name}. {} already has a different server with that name.",
                plan.title
            ),
        )
        .then(
            "Pick another name",
            ["bridge", "link", plan.program, "--name", "<NAME>"],
        ));
    }
    Err(Failure::new(
        Reason::ApiError,
        format!("{} didn't add the server.", plan.title),
    )
    .remote(plan.title, text.trim()))
}

fn register_without_signing_in(
    plan: &Plan,
    name: &str,
    url: &str,
    deadline: &Deadline,
) -> Outcome<Added> {
    let mut child = children::Owned::new(spawn(plan, &plan.register, false)?);
    let until = deadline.within(REGISTER_BUDGET);
    let phase = Phase {
        what: &format!("adding {name} to {}", plan.title),
        confirmed: format!("not known whether {} saved {name}", plan.title),
        check: Some(check_step(plan, name)),
    };
    loop {
        let details = match inspect(plan, name, deadline, &phase.confirmed) {
            Ok(details) => details,
            Err(failure) => {
                child.stop();
                return Err(failure);
            }
        };
        if details.contains(url) {
            child.stop();
            ui::done("Added", &format!("{name} to {} at {url}", plan.title));
            return Ok(Added::Added);
        }
        if Instant::now() >= until {
            child.stop();
            if deadline.end.is_some_and(|end| Instant::now() >= end) {
                return Err(stopped(plan, &plan.register, &phase, deadline));
            }
        }
        if Instant::now() >= until || child.0.try_wait()?.is_some() {
            child.stop();
            let mut argv = vec![plan.program.to_owned()];
            argv.extend(plan.register.iter().cloned());
            return Err(Failure::new(
                Reason::ApiError,
                format!("{} didn't save the server.", plan.title),
            )
            .step(Step::new("Try it yourself", argv)));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn awaiting_approval(plan: &Plan, name: &str, deadline: &Deadline) -> Outcome<bool> {
    let details = inspect(
        plan,
        name,
        deadline,
        &format!(
            "{name} was added to {}; its approval wasn't checked",
            plan.title
        ),
    )?;
    Ok(details.contains("Pending approval"))
}

struct Waiting {
    id: String,
    client_name: String,
    access: String,
}

async fn newest_request(home: &Path, seen: &BTreeSet<String>) -> Option<Waiting> {
    let pending = crate::connector::command(home, "pending", "", "")
        .await
        .ok()?;
    pending
        .as_array()?
        .iter()
        .filter_map(|request| {
            Some(Waiting {
                id: request["id"].as_str()?.to_owned(),
                client_name: request["client_name"]
                    .as_str()
                    .unwrap_or("An app")
                    .to_owned(),
                access: request["access"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .find(|request| !seen.contains(&request.id))
}

fn explain_request(request: &Waiting) {
    ui::blank();
    ui::note(&format!(
        "{} wants to connect. {}",
        ui::clean(&request.client_name),
        ui::clean(&request.access)
    ));
    ui::note(
        "Click Allow in the Apple Bridge window on your Mac. If it shows a code, check that your browser shows the same one.",
    );
    ui::next_steps(&[Step::new(
        "No window? Allow it from another terminal",
        [
            "bridge",
            "connectors",
            "allow",
            "--request",
            &request.id,
            "--code",
            "<CODE>",
        ],
    )]);
}

enum SignIn {
    Finished(bool),
    StoppedWaiting,
}

async fn sign_in(
    home: &Path,
    plan: &Plan,
    arguments: &[String],
    limit: Duration,
) -> Outcome<SignIn> {
    let child = Command::new(plan.program)
        .args(arguments)
        .stdout(Stdio::from(std::io::stderr().as_fd().try_clone_to_owned()?))
        .spawn()
        .map_err(|error| {
            Failure::new(Reason::SpawnFailed, "Couldn't start sign-in.")
                .row("Detail", ui::clean(&error.to_string()))
        })?;
    let mut child = children::Owned::new(child);
    let deadline = Instant::now() + limit;
    let mut seen = BTreeSet::new();
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(SignIn::Finished(status.success()));
        }
        if Instant::now() >= deadline {
            child.stop();
            return Ok(SignIn::StoppedWaiting);
        }
        if let Some(request) = newest_request(home, &seen).await {
            explain_request(&request);
            seen.insert(request.id);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn login_step(plan: &Plan) -> Step {
    let mut argv = vec![plan.program.to_owned()];
    argv.extend(plan.login.iter().cloned());
    Step::new("Sign in later", argv)
}

fn try_it() {
    ui::note(
        "Try it: ask your agent \"Use Apple Bridge to show my reminder lists and how much has synced.\"",
    );
    ui::next_steps(&[Step::new(
        "Give your agent the instructions",
        ["bridge", "skills", "get", "core"],
    )]);
}

fn command_json(program: &str, arguments: &[String]) -> serde_json::Value {
    let mut argv = vec![program.to_owned()];
    argv.extend(arguments.iter().cloned());
    serde_json::json!({"argv": argv, "command": shown(program, arguments)})
}

fn linked_json(
    outcome: &str,
    plan: &Plan,
    name: &str,
    url: &str,
    added: Added,
    next: &[Step],
) -> serde_json::Value {
    serde_json::json!({
        "outcome": outcome,
        "client": plan.program,
        "name": name,
        "url": url,
        "added": added == Added::Added,
        "signedIn": outcome == "connected",
        "next": next.iter().map(Step::to_json).collect::<Vec<_>>(),
    })
}

#[allow(clippy::too_many_lines)]
pub async fn link(home: &Path, args: &LinkArgs) -> Outcome {
    let given = args
        .timeout
        .as_deref()
        .map(|text| commands::duration("--timeout", text))
        .transpose()?;
    if (args.client == LinkClient::Other || args.print) && args.timeout.is_some() {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --timeout here. Nothing waits for sign-in.",
        ));
    }
    let deadline = Deadline::new(given);
    let address = gateway::running_address(home).await?;
    let url = gateway::mcp_address(&address);
    let Some(plan) = plan(args.client, &args.name, args.scope, &url, args.no_open) else {
        print_other(&url);
        return Ok(());
    };
    if args.print {
        if Modes::get().json {
            ui::json_document(serde_json::json!({
                "client": plan.program,
                "name": args.name,
                "url": url,
                "register": command_json(plan.program, &plan.register),
                "login": command_json(plan.program, &plan.login),
            }));
        } else {
            ui::outln(&shown(plan.program, &plan.register));
            ui::outln(&shown(plan.program, &plan.login));
        }
        return Ok(());
    }
    if args.client == LinkClient::Claude && !args.no_login && !Modes::get().interactive() {
        if let Some(outcome) = approve_on_mac(home, &plan, args, &url, &deadline).await {
            return outcome;
        }
    }
    let can_sign_in = !args.no_login && Modes::get().interactive();
    let mut sign_in_arguments = if plan.register_signs_in {
        plan.register.clone()
    } else {
        plan.login.clone()
    };
    let added = if plan.register_signs_in {
        if can_sign_in {
            Added::Added
        } else {
            register_without_signing_in(&plan, &args.name, &url, &deadline)?
        }
    } else {
        let added = register(&plan, &args.name, &url, &deadline)?;
        if awaiting_approval(&plan, &args.name, &deadline)? {
            ui::warn(
                "pending_approval",
                "Claude Code asks you to approve project servers first. Start it in this folder and choose \"Use this MCP server\".",
            );
            if !Modes::get().json {
                ui::next_steps(&[
                    Step::new("Approve it", [plan.program]),
                    Step::new(
                        "Then run this again",
                        [
                            "bridge",
                            "link",
                            plan.program,
                            "--scope",
                            args.scope.as_str(),
                        ],
                    ),
                ]);
            }
            piped_result(&args.name);
            if Modes::get().json {
                ui::json_document(linked_json(
                    "pending_approval",
                    &plan,
                    &args.name,
                    &url,
                    added,
                    &[Step::new("Approve it", [plan.program])],
                ));
            }
            return Ok(());
        }
        added
    };
    if !can_sign_in {
        let finish = Step {
            benefit: "To finish, run this in a terminal and approve it in the browser".to_owned(),
            ..login_step(&plan)
        };
        if Modes::get().json {
            ui::json_document(linked_json(
                "registered",
                &plan,
                &args.name,
                &url,
                added,
                &[finish, check_step(&plan, &args.name)],
            ));
            return Ok(());
        }
        piped_result(&args.name);
        ui::note(&format!(
            "{} is registered. Signing in needs a terminal and a browser.",
            plan.title
        ));
        ui::next_steps(&[finish]);
        try_it();
        return Ok(());
    }
    if !args.no_open && !Modes::get().browser_allowed() && !plan.register_signs_in {
        sign_in_arguments.push("--no-browser".to_owned());
    }
    let limit = deadline.end.map_or(SIGN_IN_WAIT, |end| {
        end.saturating_duration_since(Instant::now())
    });
    ui::note(&format!(
        "Waiting for you to sign in to {} (up to {})…",
        plan.title,
        commands::friendly(limit)
    ));
    ui::set_cancel_note(Some(&format!(
        "Canceled sign-in. {} stays registered in {}.",
        args.name, plan.title
    )));
    ui::set_cancel_step(Some(login_step(&plan)));
    match sign_in(home, &plan, &sign_in_arguments, limit).await? {
        SignIn::Finished(true) => {
            ui::set_cancel_note(None);
            ui::set_cancel_step(None);
            piped_result(&args.name);
            ui::receipt(
                "Connected",
                plan.title,
                &format!("Connected {} as {}.", plan.title, args.name),
            );
            try_it();
            Ok(())
        }
        SignIn::Finished(false) => Err(Failure::new(
            Reason::ApiError,
            format!("{} didn't finish signing in.", plan.title),
        )
        .sentence(format!("{} stays registered.", args.name))
        .step(Step {
            benefit: "Try again".to_owned(),
            ..login_step(&plan)
        })),
        SignIn::StoppedWaiting => Err(Failure::new(
            Reason::Timeout,
            format!(
                "Stopped waiting for sign-in after {}.",
                commands::friendly(limit)
            ),
        )
        .row(
            "Confirmed",
            format!("{} stays registered in {}", args.name, plan.title),
        )
        .data("confirmed", "registered; not signed in")
        .step(login_step(&plan))),
    }
}

const MAC_APPROVAL_WAIT: Duration = Duration::from_secs(300);

async fn approve_on_mac(
    home: &Path,
    plan: &Plan,
    args: &LinkArgs,
    url: &str,
    deadline: &Deadline,
) -> Option<Outcome> {
    let ticket = crate::connector::ask_for_agent(home, plan.title).await.ok()?;
    let id = ticket["id"].as_str()?.to_owned();
    let secret = ticket["secret"].as_str()?.to_owned();
    ui::note(&format!(
        "Asking on your Mac: click Allow in the Apple Bridge window to connect {}.",
        plan.title
    ));
    let until = deadline.within(MAC_APPROVAL_WAIT);
    let token = loop {
        if Instant::now() >= until {
            return Some(Err(Failure::new(
                Reason::Timeout,
                "Nobody answered on the Mac in time. Nothing was connected.",
            )
            .then("Ask again", ["bridge", "link", "claude"])));
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        let Ok(answer) = crate::connector::agent_answer(home, &id, &secret).await else {
            return Some(Err(Failure::new(
                Reason::Timeout,
                "The request on the Mac expired. Nothing was connected.",
            )
            .then("Ask again", ["bridge", "link", "claude"])));
        };
        match answer["state"].as_str() {
            Some("allowed") => break answer["token"].as_str().unwrap_or_default().to_owned(),
            Some("denied") => {
                return Some(Err(Failure::canceled(
                    "You chose Deny on the Mac. Nothing was connected.",
                )));
            }
            _ => {}
        }
    };
    if token.is_empty() {
        return Some(Err(Failure::new(Reason::ApiError, "The Mac allowed it but sent no key.")));
    }
    ui::mask_secret(&token);
    let mut keyed = Plan {
        program: plan.program,
        title: plan.title,
        install: plan.install,
        register: plan.register.clone(),
        login: plan.login.clone(),
        register_signs_in: plan.register_signs_in,
    };
    let at = keyed.register.len().saturating_sub(2);
    keyed.register.splice(
        at..at,
        ["--header".to_owned(), format!("Authorization: Bearer {token}")],
    );
    let _ = Command::new(plan.program)
        .args(["mcp", "remove", "--scope", args.scope.as_str(), &args.name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Err(failure) = register(&keyed, &args.name, url, deadline) {
        return Some(Err(failure));
    }
    piped_result(&args.name);
    if Modes::get().json {
        ui::json_document(serde_json::json!({
            "outcome": "connected",
            "client": plan.program,
            "name": args.name,
            "url": url,
            "next": "Restart Claude Code, or run /mcp, to load it.",
        }));
        return Some(Ok(()));
    }
    ui::receipt(
        "Connected",
        plan.title,
        &format!("Connected {} as {}. Restart Claude Code, or run /mcp, to load it.", plan.title, args.name),
    );
    try_it();
    Some(Ok(()))
}

pub fn unlink(args: &UnlinkArgs) -> Outcome {
    let given = args
        .timeout
        .as_deref()
        .map(|text| commands::duration("--timeout", text))
        .transpose()?;
    let deadline = Deadline::new(given);
    let Some(plan) = plan(args.client, &args.name, args.scope, "", false) else {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't unlink other. bridge unlink works with claude or codex.",
        )
        .then(
            "For other apps, see the agents with access",
            ["bridge", "agents", "list"],
        ));
    };
    run_bounded(
        &plan,
        &strings(&["mcp", "logout", &args.name]),
        &deadline,
        &Phase {
            what: &format!("signing {} out of {}", args.name, plan.title),
            confirmed: format!("{} is still registered in {}", args.name, plan.title),
            check: Some(check_step(&plan, &args.name)),
        },
    )?;
    let mut remove = strings(&["mcp", "remove", &args.name]);
    if args.client == LinkClient::Claude {
        remove.extend(strings(&["--scope", args.scope.as_str()]));
    }
    let (success, text) = run_bounded(
        &plan,
        &remove,
        &deadline,
        &Phase {
            what: &format!("removing {} from {}", args.name, plan.title),
            confirmed: format!(
                "signed out; not known whether {} removed {}",
                plan.title, args.name
            ),
            check: Some(check_step(&plan, &args.name)),
        },
    )?;
    let report = |outcome: &str, label: &str, next: &[Step]| {
        if Modes::get().json {
            ui::json_document(serde_json::json!({
                "outcome": outcome,
                "client": plan.program,
                "name": args.name,
                "next": next.iter().map(Step::to_json).collect::<Vec<_>>(),
            }));
        } else {
            piped_result(&args.name);
            let sentence = if outcome == "removed" {
                format!("Removed {} from {}.", args.name, plan.title)
            } else {
                format!("{} isn't in {}.", args.name, plan.title)
            };
            ui::receipt(
                label,
                &format!("{} from {}", args.name, plan.title),
                &sentence,
            );
            ui::next_steps(next);
        }
    };
    let find = Step::new(
        "To end its access on this Mac too, find it",
        ["bridge", "agents", "list"],
    );
    if !success {
        if text.contains("not found")
            || text.contains("No MCP server")
            || text.contains("does not exist")
        {
            report("already_removed", "Already removed", &[]);
            return Ok(());
        }
        return Err(Failure::new(
            Reason::ApiError,
            format!("{} couldn't remove {}.", plan.title, args.name),
        )
        .remote(plan.title, text.trim()));
    }
    report("removed", "Removed", &[find]);
    Ok(())
}
