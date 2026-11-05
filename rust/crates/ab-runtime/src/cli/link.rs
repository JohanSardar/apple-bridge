use super::{
    Failure, Outcome, gateway,
    tree::{LinkArgs, LinkClient, LinkScope, UnlinkArgs},
};
use std::{
    collections::BTreeSet,
    io::{BufRead, IsTerminal, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const TRY_IT: &str = "Try it: ask your agent \"How many messages, contacts and events do I have in Apple Bridge?\"\nAgent instructions: bridge skills get core";

struct Plan {
    program: &'static str,
    title: &'static str,
    install: &'static str,
    register: Vec<String>,
    login: Vec<String>,
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
            })
        }
        LinkClient::Codex => Some(Plan {
            program: "codex",
            title: "Codex",
            install: "https://developers.openai.com/codex",
            register: strings(&["mcp", "add", name, "--url", url]),
            login: strings(&["mcp", "login", name]),
        }),
        LinkClient::Other => None,
    }
}

fn shown(program: &str, arguments: &[String]) -> String {
    let quoted: Vec<String> = arguments
        .iter()
        .map(|argument| gateway::shell_quote(argument))
        .collect();
    format!("{program} {}", quoted.join(" "))
}

fn print_other(url: &str) {
    println!("Any MCP app on this Mac can use this address:");
    println!("  {url}");
    println!();
    println!("Apps that support sign-in ask you to approve them the first time.");
    println!("Bots without sign-in can use a token. Keep it secret, and give it the least access:");
    println!(
        "  bridge agent create --name my-bot --level read --scope messages:read,contacts:read"
    );
    println!("Then send it as: Authorization: Bearer <token>");
    println!(
        "REST uses the same token: POST http://{}/v1/op/sources.status",
        url.trim_start_matches("http://").trim_end_matches("/mcp")
    );
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
                Failure::new(format!("Could not run {}: {error}.", plan.program))
            }
        })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text))
}

fn missing(plan: &Plan) -> Failure {
    Failure::new(format!(
        "{} is not installed, or `{}` is not on your PATH.",
        plan.title, plan.program
    ))
    .next(format!(
        "Install it from {}, then run this again. For other apps: bridge link other",
        plan.install
    ))
}

fn register(plan: &Plan, name: &str, url: &str) -> Outcome {
    let (success, text) = run_quietly(plan, &plan.register)?;
    if success {
        println!("Added {name} to {} at {url}.", plan.title);
        return Ok(());
    }
    if text.contains("already exists") {
        let (_, details) = run_quietly(plan, &strings(&["mcp", "get", name]))?;
        if details.contains(url) {
            println!("{} already has {name} at {url}.", plan.title);
            return Ok(());
        }
        return Err(Failure::new(format!(
            "{} already has a different server called {name}.",
            plan.title
        ))
        .next("Pick another name with --name, or remove it with `bridge unlink`."));
    }
    Err(Failure::new(format!(
        "{} refused the new server: {}",
        plan.title,
        text.trim()
    )))
}

fn stdin_lines() -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    receiver
}

fn say(text: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
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

async fn approve_from_terminal(home: &Path, request: &Waiting, code: &str) -> bool {
    crate::connector::command(home, "allow", &request.id, code.trim())
        .await
        .is_ok()
}

fn explain_request(home: &Path, request: &Waiting, can_type: bool) {
    let home_flag = if crate::default_home().is_ok_and(|default| default == home) {
        String::new()
    } else {
        format!(" --home {}", gateway::shell_quote(&home.to_string_lossy()))
    };
    say("");
    say(&format!(
        "{} wants to connect. {}",
        request.client_name, request.access
    ));
    if can_type {
        say("Type the four-digit code shown in your browser to allow it. Press Enter to skip:");
    } else {
        say("Allow it, using the four-digit code shown in your browser:");
        say(&format!(
            "  bridge connector allow --request {} --code CODE{home_flag}",
            request.id
        ));
    }
}

async fn login(home: &Path, plan: &Plan, args: &LinkArgs) -> Outcome<bool> {
    let can_type = !args.no_browser && std::io::stdin().is_terminal();
    let mut command = Command::new(plan.program);
    command.args(&plan.login);
    if !args.no_browser {
        command.stdin(Stdio::null());
    }
    let mut child = command
        .spawn()
        .map_err(|error| Failure::new(format!("Could not start sign-in: {error}.")))?;
    let typed = can_type.then(stdin_lines);
    let mut seen = BTreeSet::new();
    let mut current: Option<Waiting> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if current.is_none()
            && let Some(request) = newest_request(home, &seen).await
        {
            explain_request(home, &request, can_type);
            seen.insert(request.id.clone());
            current = Some(request);
        }
        if let (Some(lines), Some(request)) = (&typed, &current)
            && let Ok(line) = lines.try_recv()
        {
            if !line.trim().is_empty() {
                if approve_from_terminal(home, request, &line).await {
                    say("Allowed. Finishing sign-in…");
                } else {
                    say(
                        "That code did not match. Check the browser and allow it with `bridge connector allow`.",
                    );
                }
            }
            current = None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub async fn link(home: &Path, args: &LinkArgs) -> Outcome {
    let address = gateway::running_address(home).await?;
    let url = gateway::mcp_address(&address);
    let Some(plan) = plan(args.client, &args.name, args.scope, &url, args.no_browser) else {
        print_other(&url);
        return Ok(());
    };
    if args.print {
        println!("{}", shown(plan.program, &plan.register));
        println!("{}", shown(plan.program, &plan.login));
        return Ok(());
    }
    register(&plan, &args.name, &url)?;
    let wants_login = !args.no_login && (args.no_browser || std::io::stdin().is_terminal());
    if !wants_login {
        println!("To finish, run this in a terminal and approve it in the browser:");
        println!("  {}", shown(plan.program, &plan.login));
        println!("{TRY_IT}");
        return Ok(());
    }
    if login(home, &plan, args).await? {
        println!("{} is connected.", plan.title);
        println!("{TRY_IT}");
        Ok(())
    } else {
        Err(
            Failure::new(format!("{} did not finish signing in.", plan.title)).next(format!(
                "Try again with: {}",
                shown(plan.program, &plan.login)
            )),
        )
    }
}

pub fn unlink(args: &UnlinkArgs) -> Outcome {
    let Some(plan) = plan(args.client, &args.name, args.scope, "", false) else {
        return Err(Failure::new("`bridge unlink` works with claude or codex.")
            .next("For other apps, run `bridge agent list` and `bridge agent revoke`."));
    };
    run_quietly(&plan, &strings(&["mcp", "logout", &args.name]))?;
    let mut remove = strings(&["mcp", "remove", &args.name]);
    if args.client == LinkClient::Claude {
        remove.extend(strings(&["--scope", args.scope.as_str()]));
    }
    let (success, text) = run_quietly(&plan, &remove)?;
    if !success {
        return Err(Failure::new(format!(
            "{} could not remove {}: {}",
            plan.title,
            args.name,
            text.trim()
        )));
    }
    println!("Removed {} from {}.", args.name, plan.title);
    println!(
        "To end its access on this Mac too: bridge agent list, then bridge agent revoke <id>."
    );
    Ok(())
}
