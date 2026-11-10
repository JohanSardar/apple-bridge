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
const TRY_IT: &str = "Try it: ask your agent \"Use Apple Bridge to show my reminder lists. Then run bridge stats and tell me how much has synced.\"\nAgent instructions: bridge skills get core";

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
    no_browser: bool,
) -> Option<Plan> {
    match client {
        LinkClient::Claude => {
            let mut login = strings(&["mcp", "login", name]);
            if no_browser {
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
        LinkClient::Other => None,
    }
}

fn shown(program: &str, arguments: &[String]) -> String {
    let quoted: Vec<String> = arguments
        .iter()
        .map(|argument| ui::shell_quote(argument))
        .collect();
    format!("{program} {}", quoted.join(" "))
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

fn run_quietly(plan: &Plan, arguments: &[String]) -> Outcome<(bool, String)> {
    let output = Command::new(plan.program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                missing(plan)
            } else {
                Failure::new(
                    Reason::SpawnFailed,
                    format!("Couldn't run {}.", plan.program),
                )
                .row("Detail", ui::clean(&error.to_string()))
            }
        })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text))
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

fn register(plan: &Plan, name: &str, url: &str) -> Outcome {
    let (success, text) = run_quietly(plan, &plan.register)?;
    if success {
        ui::done("Added", &format!("{name} to {} at {url}", plan.title));
        return Ok(());
    }
    if text.contains("already exists") {
        let (_, details) = run_quietly(plan, &strings(&["mcp", "get", name]))?;
        if details.contains(url) {
            ui::done(
                "Already added",
                &format!("{name} to {} at {url}", plan.title),
            );
            return Ok(());
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

fn register_without_signing_in(plan: &Plan, name: &str, url: &str) -> Outcome {
    let child = Command::new(plan.program)
        .args(&plan.register)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                missing(plan)
            } else {
                Failure::new(
                    Reason::SpawnFailed,
                    format!("Couldn't run {}.", plan.program),
                )
                .row("Detail", ui::clean(&error.to_string()))
            }
        })?;
    let mut child = children::Owned::new(child);
    let deadline = Instant::now() + REGISTER_BUDGET;
    loop {
        let (_, details) = run_quietly(plan, &strings(&["mcp", "get", name]))?;
        if details.contains(url) {
            child.stop();
            ui::done("Added", &format!("{name} to {} at {url}", plan.title));
            return Ok(());
        }
        if Instant::now() >= deadline || child.0.try_wait()?.is_some() {
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

fn awaiting_approval(plan: &Plan, name: &str) -> Outcome<bool> {
    let (_, details) = run_quietly(plan, &strings(&["mcp", "get", name]))?;
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
        "Check the four-digit code in your browser, then allow it in the Apple Bridge window.",
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

pub async fn link(home: &Path, args: &LinkArgs) -> Outcome {
    let limit = args
        .timeout
        .as_deref()
        .map(|text| commands::duration("--timeout", text))
        .transpose()?
        .unwrap_or(SIGN_IN_WAIT);
    if (args.client == LinkClient::Other || args.print) && args.timeout.is_some() {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --timeout here. Nothing waits for sign-in.",
        ));
    }
    let address = gateway::running_address(home).await?;
    let url = gateway::mcp_address(&address);
    let no_browser = args.no_open || (!args.print && !Modes::get().browser_allowed());
    let Some(plan) = plan(args.client, &args.name, args.scope, &url, no_browser) else {
        print_other(&url);
        return Ok(());
    };
    if args.print {
        ui::outln(&shown(plan.program, &plan.register));
        ui::outln(&shown(plan.program, &plan.login));
        return Ok(());
    }
    let can_sign_in = !args.no_login && Modes::get().interactive();
    let sign_in_arguments = if plan.register_signs_in {
        if !can_sign_in {
            register_without_signing_in(&plan, &args.name, &url)?;
        }
        &plan.register
    } else {
        register(&plan, &args.name, &url)?;
        if awaiting_approval(&plan, &args.name)? {
            ui::warn(
                "pending_approval",
                "Claude Code asks you to approve project servers first: run claude in this folder and choose \"Use this MCP server\", then run this command again.",
            );
            return Ok(());
        }
        &plan.login
    };
    if !can_sign_in {
        ui::note(&format!(
            "{} is registered. Signing in needs a terminal and a browser.",
            plan.title
        ));
        ui::next_steps(&[Step {
            benefit: "To finish, run this in a terminal and approve it in the browser".to_owned(),
            ..login_step(&plan)
        }]);
        ui::note(TRY_IT);
        return Ok(());
    }
    ui::note(&format!(
        "Waiting for you to sign in to {} (up to {})…",
        plan.title,
        commands::friendly(limit)
    ));
    ui::set_cancel_note(Some(&format!(
        "Canceled sign-in. {} stays registered in {}; sign in later with: {}",
        args.name,
        plan.title,
        login_step(&plan).command()
    )));
    match sign_in(home, &plan, sign_in_arguments, limit).await? {
        SignIn::Finished(true) => {
            ui::set_cancel_note(None);
            ui::done("Connected", plan.title);
            ui::note(TRY_IT);
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
        .sentence(format!("{} stays registered in {}.", args.name, plan.title))
        .step(login_step(&plan))),
    }
}

pub fn unlink(args: &UnlinkArgs) -> Outcome {
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
    run_quietly(&plan, &strings(&["mcp", "logout", &args.name]))?;
    let mut remove = strings(&["mcp", "remove", &args.name]);
    if args.client == LinkClient::Claude {
        remove.extend(strings(&["--scope", args.scope.as_str()]));
    }
    let (success, text) = run_quietly(&plan, &remove)?;
    if !success {
        if text.contains("not found")
            || text.contains("No MCP server")
            || text.contains("does not exist")
        {
            ui::done(
                "Already removed",
                &format!("{} from {}", args.name, plan.title),
            );
            return Ok(());
        }
        return Err(Failure::new(
            Reason::ApiError,
            format!("{} couldn't remove {}.", plan.title, args.name),
        )
        .remote(plan.title, text.trim()));
    }
    ui::done("Removed", &format!("{} from {}", args.name, plan.title));
    ui::next_steps(&[Step::new(
        "To end its access on this Mac too, find it",
        ["bridge", "agents", "list"],
    )]);
    Ok(())
}
