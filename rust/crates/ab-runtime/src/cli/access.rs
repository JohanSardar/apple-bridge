use super::{
    Failure, Outcome,
    failure::{Reason, Step},
    modes::Modes,
    tree::{AgentCreateArgs, DecisionArgs},
    ui,
};
use crate::product_cli;
use ab_trust::{AgentLevel, Authority, Grant, Principal};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static LEGACY_REVOKE: AtomicBool = AtomicBool::new(false);

pub fn typed_as(words: &[String]) {
    let legacy = matches!(words.first().map(String::as_str), Some("agent" | "revoke"));
    LEGACY_REVOKE.store(legacy, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selector {
    Agent,
    Any,
}

pub fn open(home: &Path) -> Outcome<Authority> {
    Authority::open(home).map_err(|error| match error {
        ab_trust::Error::CredentialStore(failure) => Failure::new(
            Reason::PermissionDenied,
            "Can't open this home's credentials. The credential store refused.",
        )
        .row("Store", failure.store.clone())
        .sentence(ui::clean(&failure.remedy)),
        other => Failure::from(other),
    })
}

fn existing(home: &Path) -> Outcome<Option<Authority>> {
    if home.join("credentials.json").is_file() {
        open(home).map(Some)
    } else {
        Ok(None)
    }
}

pub fn parse_grants(scope: &str, for_agent: bool) -> Outcome<Vec<Grant>> {
    let mut grants = Vec::new();
    let mut issues = Vec::new();
    for name in scope.split(',').map(str::trim) {
        if name.is_empty() {
            issues.push(
                "An empty grant isn't allowed; separate grants with single commas.".to_owned(),
            );
            continue;
        }
        match Grant::parse(name) {
            Ok(grant) => {
                let source = name.split_once(':').map_or(name, |(source, _)| source);
                if for_agent && !ab_trust::agent_source(source) {
                    issues.push(format!("{} isn't available to agents.", ui::clean(name)));
                } else {
                    grants.push(grant);
                }
            }
            Err(_) => issues.push(format!("{} isn't a known grant.", ui::clean(name))),
        }
    }
    if issues.is_empty() {
        return Ok(grants);
    }
    let count = issues.len();
    let mut failure = Failure::new(
        Reason::InvalidValue,
        format!(
            "Can't use --scope {}. {count} {} invalid.",
            ui::clean(scope),
            if count == 1 { "grant is" } else { "grants are" }
        ),
    )
    .sentence("Grants look like messages:read or calendar:write.");
    for issue in issues {
        failure = failure.issue(issue);
    }
    Err(failure)
}

fn level(text: &str, switchable: bool) -> Outcome<AgentLevel> {
    match AgentLevel::parse(text) {
        Some(AgentLevel::Read) if switchable => Err(Failure::new(
            Reason::InvalidValue,
            "Can't switch an agent to read. Accepted: full, approve.",
        )
        .sentence("For read-only access, revoke the agent and create a read agent.")),
        Some(level) => Ok(level),
        None => Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --level {}. Accepted: {}.",
                ui::clean(text),
                if switchable {
                    "full, approve"
                } else {
                    "full, approve, read"
                }
            ),
        )),
    }
}

fn check_agent_id(id: &str) -> Outcome {
    let valid = id
        .strip_prefix("agent:")
        .is_some_and(|rest| rest.len() == 26 && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if valid {
        Ok(())
    } else {
        Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {} as an agent ID. Agent IDs look like agent:01J….",
                ui::clean(id)
            ),
        )
        .then("List agents", ["bridge", "agents", "list"]))
    }
}

fn check_credential_id(id: &str) -> Outcome {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":_-.".contains(c));
    if valid {
        Ok(())
    } else {
        Err(Failure::new(
            Reason::InvalidValue,
            format!("Can't use {} as a credential ID.", ui::clean(id)),
        )
        .then("List agents", ["bridge", "agents", "list"]))
    }
}

fn not_found(id: &str) -> Failure {
    Failure::new(
        Reason::NotFound,
        format!("Couldn't find credential {}.", ui::clean(id)),
    )
    .then("List agents", ["bridge", "agents", "list"])
}

#[derive(serde::Serialize)]
struct AgentSummary<'a> {
    id: &'a str,
    name: &'a str,
    level: &'a str,
    scopes: &'a [Grant],
    created_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used: Option<&'a Value>,
}

fn summary(agent: &Principal) -> AgentSummary<'_> {
    let text = |key: &str| {
        agent
            .extra
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let present = |key: &str| agent.extra.get(key).filter(|value| !value.is_null());
    AgentSummary {
        id: &agent.id,
        name: text("name"),
        level: &agent.level,
        scopes: &agent.grants,
        created_at: text("created_at"),
        revoked_at: present("revoked_at"),
        last_used: present("last_used"),
    }
}

fn summary_json(agent: &Principal) -> Value {
    serde_json::to_value(summary(agent)).unwrap_or(Value::Null)
}

fn access_text(grants: &[Grant]) -> String {
    grants
        .iter()
        .map(Grant::as_str)
        .collect::<Vec<_>>()
        .join(",")
}

fn legacy_notice(command: &str) {
    ui::warn(
        "deprecated",
        &format!(
            "Piped {command} prints JSON for one more release, then a table. Pass --json to keep JSON."
        ),
    );
}

pub fn agents_list(home: &Path) -> Outcome {
    let agents = match existing(home)? {
        Some(authority) => authority.agents()?,
        None => Vec::new(),
    };
    let documents: Vec<AgentSummary<'_>> = agents.iter().map(summary).collect();
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy_notice("bridge agents list");
        }
        if modes.json && modes.fields.is_some() {
            ui::json_document(serde_json::to_value(&documents)?);
        } else {
            ui::exact(&documents);
        }
        return Ok(());
    }
    if agents.is_empty() {
        ui::note("No agents on this Mac.");
        ui::next_steps(&[Step::new(
            "Create one",
            ["bridge", "agents", "create", "--name", "<NAME>"],
        )]);
        if !modes.stdout_table() {
            ui::table(
                &["ID", "Name", "Level", "Access", "Created", "Revoked"],
                &[],
            );
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = agents
        .iter()
        .map(|agent| {
            let shown = summary(agent);
            let when = |value: &str| {
                if modes.stdout_table() {
                    ui::age(value)
                } else {
                    value.to_owned()
                }
            };
            vec![
                shown.id.to_owned(),
                shown.name.to_owned(),
                shown.level.to_owned(),
                access_text(shown.scopes),
                when(shown.created_at),
                shown
                    .revoked_at
                    .and_then(Value::as_str)
                    .map_or_else(|| "-".to_owned(), when),
            ]
        })
        .collect();
    ui::table(
        &["ID", "Name", "Level", "Access", "Created", "Revoked"],
        &rows,
    );
    Ok(())
}

fn find_agent(home: &Path, id: &str) -> Outcome<(Authority, Principal)> {
    let authority = existing(home)?.ok_or_else(|| not_found(id))?;
    let agent = authority
        .agents()?
        .into_iter()
        .find(|agent| agent.id == id)
        .ok_or_else(|| not_found(id))?;
    Ok((authority, agent))
}

fn agent_rows(agent: &Principal) {
    let shown = summary(agent);
    ui::row("Name", shown.name);
    ui::row("Level", shown.level);
    ui::row("Access", &access_text(shown.scopes));
}

pub fn agents_inspect(home: &Path, id: &str) -> Outcome {
    check_agent_id(id)?;
    let (_, agent) = find_agent(home, id)?;
    if Modes::get().json {
        ui::json_document(summary_json(&agent));
        return Ok(());
    }
    let shown = summary(&agent);
    let lines = [
        ("Agent", shown.id.to_owned()),
        ("Name", shown.name.to_owned()),
        ("Level", shown.level.to_owned()),
        ("Access", access_text(shown.scopes)),
        ("Created", shown.created_at.to_owned()),
        (
            "Revoked",
            shown
                .revoked_at
                .and_then(Value::as_str)
                .unwrap_or("no")
                .to_owned(),
        ),
        (
            "Last used",
            shown.last_used.map_or_else(
                || "never".to_owned(),
                |value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                },
            ),
        ),
    ];
    for (label, value) in lines {
        ui::outln(&format!("{label:<16}{}", ui::clean(&value)));
    }
    Ok(())
}

pub fn agents_create(home: &Path, args: &AgentCreateArgs) -> Outcome {
    let name = args.name.trim();
    let mut problems = Vec::new();
    if name.is_empty() || name.len() > 120 {
        problems.push(Failure::new(
            Reason::InvalidValue,
            "Can't use that --name. Names are 1 to 120 bytes; accented letters and emoji take 2 to 4 each.",
        ));
    }
    let level = if args.level.is_empty() {
        Ok(AgentLevel::DEFAULT)
    } else {
        level(&args.level, false)
    };
    let grants = parse_grants(&args.scope, true);
    let (level, grants) = match (level, grants) {
        (Ok(level), Ok(grants)) if problems.is_empty() => (level, grants),
        (level, grants) => {
            problems.extend(level.err());
            problems.extend(grants.err());
            return Err(combine("Can't create that agent.", problems));
        }
    };
    let authority = open(home)?;
    let (agent, token) =
        authority
            .create_agent(name, level, grants)
            .map_err(|error| match error {
                ab_trust::Error::Unauthorized => Failure::new(
                    Reason::PermissionDenied,
                    "Can't create that agent. This home can only grant the access it was given.",
                ),
                other => Failure::from(other),
            })?;
    ui::mask_secret(&token);
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        #[derive(serde::Serialize)]
        struct Created<'a> {
            agent: AgentSummary<'a>,
            token: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            outcome: Option<&'a str>,
        }
        let created = Created {
            agent: summary(&agent),
            token: &token,
            outcome: modes.json.then_some("created"),
        };
        if !modes.json {
            legacy_notice("bridge agents create");
        }
        if modes.fields.is_some() {
            ui::json_document(serde_json::to_value(&created)?);
        } else {
            ui::exact(&created);
        }
        return Ok(());
    }
    ui::outln(&token);
    ui::done("Created", &agent.id);
    agent_rows(&agent);
    ui::note("The token above is shown once. Keep it secret.");
    Ok(())
}

pub fn agents_update(
    home: &Path,
    id: &str,
    new_level: Option<&str>,
    scope: Option<&str>,
) -> Outcome {
    check_agent_id(id)?;
    if new_level.is_none() && scope.is_none() {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't update an agent without --level or --scope.",
        )
        .then(
            "Change its level",
            ["bridge", "agents", "update", id, "--level", "<LEVEL>"],
        ));
    }
    let chosen = new_level.map(|text| level(text, true)).transpose()?;
    let grants = scope.map(|scope| parse_grants(scope, true)).transpose()?;
    let (authority, before) = find_agent(home, id)?;
    if !before.active() {
        return Err(Failure::new(
            Reason::Conflict,
            format!("Can't update {id}. It's revoked."),
        )
        .then(
            "Create a new agent",
            ["bridge", "agents", "create", "--name", "<NAME>"],
        ));
    }
    let unchanged_level = chosen.is_none_or(|level| level.as_str() == before.level);
    let unchanged_grants = grants
        .as_ref()
        .is_none_or(|grants| access_text(grants) == access_text(&before.grants));
    let mut after = before.clone();
    if let Some(level) = chosen
        && !unchanged_level
    {
        after = authority
            .set_agent_level(id, level)
            .map_err(|error| match error {
                ab_trust::Error::Invalid => Failure::new(
                    Reason::ReadOnly,
                    format!("Can't change the level of {id}. It's a read agent."),
                )
                .sentence("Revoke it and create a new agent to give it write access."),
                _ => not_found(id),
            })?;
    }
    if let Some(grants) = grants
        && !unchanged_grants
    {
        after = authority.set_grants(id, grants).map_err(|_| {
            Failure::new(
                Reason::PermissionDenied,
                format!("Can't change the access of {id}. This home can't grant it."),
            )
        })?;
    }
    report_agent_change(&before, &after, unchanged_level && unchanged_grants)
}

#[allow(clippy::unnecessary_wraps)]
fn report_agent_change(before: &Principal, after: &Principal, unchanged: bool) -> Outcome {
    let outcome = if unchanged { "unchanged" } else { "updated" };
    let modes = Modes::get();
    if modes.json {
        ui::json_document(json!({"agent": summary_json(after), "outcome": outcome}));
        return Ok(());
    }
    if modes.legacy_json() {
        legacy_notice("bridge agents level");
        ui::exact(&summary(after));
        return Ok(());
    }
    if unchanged {
        ui::done("Already set", &after.id);
    } else {
        ui::done("Updated", &after.id);
    }
    if before.level == after.level {
        ui::row("Level", &after.level);
    } else {
        ui::row("Level", &format!("{} → {}", before.level, after.level));
    }
    ui::row("Access", &access_text(&after.grants));
    Ok(())
}

pub fn agents_level(home: &Path, id: &str, text: &str) -> Outcome {
    agents_update(home, id, Some(text), None)
}

pub fn tokens_create(home: &Path, scope: &str, remote: bool) -> Outcome {
    let grants = parse_grants(scope, false)?;
    let authority = open(home)?;
    let token = authority
        .create_token(grants.clone(), remote)
        .map_err(|_| {
            Failure::new(
                Reason::PermissionDenied,
                "Can't create that token. This home can only grant the access it was given.",
            )
        })?;
    ui::mask_secret(&token);
    let id = authority
        .authenticate_token(&token)
        .map(|principal| principal.id)
        .unwrap_or_default();
    if Modes::get().json {
        ui::json_document(
            json!({"token": token, "id": id, "scopes": access_text(&grants).split(',').collect::<Vec<_>>(), "remote": remote, "outcome": "created"}),
        );
        return Ok(());
    }
    ui::outln(&token);
    if Modes::get().stdout_tty() {
        ui::done("Created", &id);
        ui::row("Access", &access_text(&grants));
        if remote {
            ui::row("Remote", "allowed");
        }
        ui::note("The token above is shown once. Keep it secret.");
    }
    Ok(())
}

fn credential(authority: &Authority, id: &str) -> Outcome<Option<Principal>> {
    Ok(authority
        .load()?
        .principals
        .into_iter()
        .find(|principal| principal.id == id))
}

pub fn revoke(home: &Path, id: &str, selector: Selector) -> Outcome {
    if selector == Selector::Agent {
        check_agent_id(id)?;
    } else {
        check_credential_id(id)?;
    }
    let authority = existing(home)?.ok_or_else(|| not_found(id))?;
    let Some(principal) = credential(&authority, id)? else {
        return Err(not_found(id));
    };
    let name = principal
        .extra
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let noun = match principal.kind.as_str() {
        "agent" => "agent",
        "token" => "token",
        "replica" => "replica",
        _ => "credential",
    };
    if !principal.active() {
        if Modes::get().json {
            ui::json_document(json!({"id": id, "kind": noun, "outcome": "unchanged"}));
        } else {
            ui::done("Already revoked", id);
        }
        return Ok(());
    }
    let shown = if name.is_empty() {
        format!("{noun} {id}")
    } else {
        format!("{noun} {}", ui::clean(&name))
    };
    let preview = vec![
        format!(
            "Revokes {id}{}.",
            if name.is_empty() {
                String::new()
            } else {
                format!(" ({})", ui::clean(&name))
            }
        ),
        format!("Access lost: {}.", access_text(&principal.grants)),
        "Anything using its token stops working at once. This cannot be undone.".to_owned(),
    ];
    let modes = Modes::get();
    if !modes.yes && !modes.interactive() && !modes.json && LEGACY_REVOKE.load(Ordering::Relaxed) {
        ui::warn(
            "deprecated",
            "Revoking without --yes outside a terminal is deprecated. Pass --yes; the next release will refuse.",
        );
    } else {
        let mut argv = vec![
            "bridge".to_owned(),
            "tokens".to_owned(),
            "revoke".to_owned(),
            id.to_owned(),
            "--yes".to_owned(),
        ];
        if selector == Selector::Agent {
            "agents".clone_into(&mut argv[1]);
        }
        ui::confirm(
            &preview,
            &format!("Revoke {shown}?"),
            &Step::new("Revoke it", argv),
            "Nothing was revoked.",
        )?;
    }
    authority.revoke(id).map_err(|_| not_found(id))?;
    if modes.json {
        ui::json_document(json!({"id": id, "kind": noun, "outcome": "revoked"}));
        return Ok(());
    }
    ui::done("Revoked", id);
    if !name.is_empty() {
        ui::row("Name", &name);
    }
    ui::row("Access", &access_text(&principal.grants));
    Ok(())
}

pub fn update_grants(home: &Path, id: &str, scope: &str) -> Outcome {
    check_credential_id(id)?;
    let for_agent = id.starts_with("agent:");
    let grants = parse_grants(scope, for_agent)?;
    let authority = existing(home)?.ok_or_else(|| not_found(id))?;
    let Some(before) = credential(&authority, id)? else {
        return Err(not_found(id));
    };
    if !before.active() {
        return Err(Failure::new(
            Reason::Conflict,
            format!("Can't change the access of {id}. It's revoked."),
        )
        .then(
            "Create a new one",
            ["bridge", "agents", "create", "--name", "<NAME>"],
        ));
    }
    if access_text(&before.grants) == access_text(&grants) {
        if Modes::get().json {
            ui::json_document(json!({"id": id, "scopes": before.grants, "outcome": "unchanged"}));
        } else {
            ui::done("Already set", id);
            ui::row("Access", &access_text(&before.grants));
        }
        return Ok(());
    }
    let after = authority
        .set_grants(id, grants)
        .map_err(|error| match error {
            ab_trust::Error::Unauthorized => not_found(id),
            _ => Failure::new(
                Reason::PermissionDenied,
                format!("Can't change the access of {id}. This home can't grant it."),
            ),
        })?;
    if Modes::get().json {
        ui::json_document(json!({"id": id, "scopes": after.grants, "outcome": "updated"}));
        return Ok(());
    }
    ui::done("Updated", id);
    ui::row(
        "Access",
        &format!(
            "{} → {}",
            access_text(&before.grants),
            access_text(&after.grants)
        ),
    );
    Ok(())
}

fn connector_unreachable(home: &Path) -> Failure {
    if home.join("run/connectors.sock").exists() {
        Failure::new(
            Reason::NetworkError,
            "Couldn't reach the gateway's approval service.",
        )
        .then("Restart Apple Bridge", ["bridge", "up"])
    } else {
        Failure::not_running(home)
    }
}

pub async fn connectors_list(home: &Path) -> Outcome {
    let pending = crate::connector::command(home, "pending", "", "")
        .await
        .map_err(|_| connector_unreachable(home))?;
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy_notice("bridge connectors list");
        }
        ui::exact_json(&pending);
        return Ok(());
    }
    let items = pending.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        ui::note("No apps are waiting to connect.");
        if !modes.stdout_table() {
            ui::table(&["Request", "App", "Redirect", "Access", "Expires"], &[]);
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            let text = |key: &str| item[key].as_str().unwrap_or_default().to_owned();
            vec![
                text("id"),
                text("client_name"),
                text("redirect_host"),
                text("access"),
                text("expires_at"),
            ]
        })
        .collect();
    ui::table(&["Request", "App", "Redirect", "Access", "Expires"], &rows);
    if let Some(first) = items.first().and_then(|item| item["id"].as_str()) {
        ui::next_steps(&[Step::new(
            "Allow it after checking the code on the app's page",
            [
                "bridge",
                "connectors",
                "allow",
                "--request",
                first,
                "--code",
                "<CODE>",
            ],
        )]);
    }
    Ok(())
}

pub async fn connectors_decide(home: &Path, decision: &DecisionArgs, allow: bool) -> Outcome {
    let valid_request = !decision.request.is_empty()
        && decision.request.len() <= 128
        && decision
            .request
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-:".contains(c));
    if !valid_request {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --request {}. Request IDs come from bridge connectors list.",
                ui::clean(&decision.request)
            ),
        ));
    }
    if decision.code.is_empty()
        || decision.code.len() > 16
        || !decision.code.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't use that --code. Copy the code shown on the app's page.",
        ));
    }
    let pending = crate::connector::command(home, "pending", "", "")
        .await
        .map_err(|_| connector_unreachable(home))?;
    let request = pending.as_array().and_then(|items| {
        items
            .iter()
            .find(|item| item["id"] == decision.request.as_str())
            .cloned()
    });
    let action = if allow { "allow" } else { "deny" };
    let result = crate::connector::command(home, action, &decision.request, &decision.code)
        .await
        .map_err(|_| {
            if request.is_none() {
                Failure::new(
                    Reason::NotFound,
                    format!("Couldn't find request {}. It may have expired.", ui::clean(&decision.request)),
                )
                .then("See the apps waiting", ["bridge", "connectors", "list"])
            } else {
                Failure::new(
                    Reason::PermissionDenied,
                    "Can't decide that request. The code doesn't match.",
                )
                .sentence("Check the four-digit code on the app's page. After 5 wrong codes the request is refused.")
            }
        })?;
    let modes = Modes::get();
    let outcome = if allow { "allowed" } else { "denied" };
    if modes.json {
        let mut document = result.clone();
        if let Value::Object(map) = &mut document {
            map.insert("outcome".into(), json!(outcome));
        }
        ui::json_document(document);
        return Ok(());
    }
    if modes.legacy_json() {
        legacy_notice(&format!("bridge connectors {action}"));
        ui::exact_json(&result);
        return Ok(());
    }
    let app = request
        .as_ref()
        .and_then(|item| item["client_name"].as_str())
        .unwrap_or("The app")
        .to_owned();
    ui::done(if allow { "Allowed" } else { "Denied" }, &app);
    ui::row("Request", &decision.request);
    if allow && let Some(access) = request.as_ref().and_then(|item| item["access"].as_str()) {
        ui::row("Access", access);
    }
    Ok(())
}

fn remote_url(home: &Path) -> Outcome<String> {
    let off = || {
        Failure::new(
            Reason::Conflict,
            "Can't use the remote address. Remote access is off.",
        )
        .then("Turn it on", ["bridge", "remote", "enable"])
    };
    let status: Value = ab_remote::state::read(home, "status").map_err(|_| off())?;
    if status["state"] != "on" {
        return Err(off());
    }
    status["url"].as_str().map(str::to_owned).ok_or_else(off)
}

fn valid_web_host(host: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(host) else {
        return false;
    };
    let Some((_, rest)) = host.split_once("://") else {
        return false;
    };
    url.host_str().is_some()
        && !rest.chars().any(|c| matches!(c, '/' | '?' | '#' | '@'))
        && (url.scheme() == "https"
            || url.scheme() == "http"
                && url.host_str() == Some("localhost")
                && rest.split(':').next() == Some("localhost"))
}

pub fn web_enroll(home: &Path, host: &str, public: bool) -> Outcome {
    let host = if public {
        let url = remote_url(home)?;
        if !url.starts_with("https://") {
            return Err(Failure::new(
                Reason::Conflict,
                "Can't use the remote address. It isn't HTTPS yet.",
            )
            .then("Check remote access", ["bridge", "remote", "status"]));
        }
        url
    } else {
        host.to_owned()
    };
    if !valid_web_host(&host) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --host {}. Use https://HOST, or http://localhost:PORT on this Mac.",
                ui::clean(&host)
            ),
        ));
    }
    let authority = open(home)?;
    let enrollment = format!("{host}/enroll#{}", authority.create_web_enroll()?);
    ui::mask_secret(&enrollment);
    if Modes::get().json {
        ui::json_document(
            json!({"url": enrollment, "expires_in_seconds": 600, "outcome": "created"}),
        );
        return Ok(());
    }
    ui::outln(&enrollment);
    if Modes::get().stdout_tty() {
        ui::out(&product_cli::terminal_qr(&enrollment)?);
        ui::note("Open the link or scan the code to add a passkey. It works once, for 10 minutes.");
    }
    Ok(())
}

pub fn sessions_list(home: &Path) -> Outcome {
    let sessions = match existing(home)? {
        Some(authority) => authority.device_sessions()?,
        None => Vec::new(),
    };
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy_notice("bridge web sessions list");
        }
        ui::exact_json(&serde_json::to_value(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        ui::note("No browsers are signed in.");
        if !modes.stdout_table() {
            ui::table(
                &["Device", "Kind", "Signed in", "Last seen", "Expires"],
                &[],
            );
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|session| {
            let when = |value: &str| {
                if modes.stdout_table() {
                    ui::age(value)
                } else {
                    value.to_owned()
                }
            };
            vec![
                session.device_id.clone(),
                session.kind.clone(),
                when(&session.created_at),
                when(&session.last_seen),
                session.expires_at.clone(),
            ]
        })
        .collect();
    ui::table(
        &["Device", "Kind", "Signed in", "Last seen", "Expires"],
        &rows,
    );
    Ok(())
}

pub fn sessions_revoke(home: &Path) -> Outcome {
    let Some(authority) = existing(home)? else {
        return already_signed_out();
    };
    let count = authority.device_sessions()?.len();
    let devices = authority
        .load()?
        .principals
        .iter()
        .filter(|principal| principal.kind == "device")
        .count();
    if count == 0 && devices == 0 {
        return already_signed_out();
    }
    let plural = if count == 1 { "browser" } else { "browsers" };
    ui::confirm(
        &[
            format!(
                "Signs out {count} {plural} from this Mac's pages and removes their passkey sessions."
            ),
            "Each browser needs a new passkey enrollment to sign in again.".to_owned(),
        ],
        &format!("Sign out {count} {plural}?"),
        &Step::new(
            "Sign them out",
            ["bridge", "web", "sessions", "revoke", "--all", "--yes"],
        ),
        "Nobody was signed out.",
    )?;
    authority.lock_web()?;
    if Modes::get().json {
        ui::json_document(json!({"revoked": count, "outcome": "revoked"}));
    } else {
        ui::done("Signed out", &format!("{count} {plural}"));
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn already_signed_out() -> Outcome {
    if Modes::get().json {
        ui::json_document(json!({"revoked": 0, "outcome": "unchanged"}));
    } else {
        ui::done("Already signed out", "no browser sessions were open");
    }
    Ok(())
}

pub fn combine(headline: &str, mut problems: Vec<Failure>) -> Failure {
    if problems.len() == 1 {
        return problems.remove(0);
    }
    let reason = problems
        .first()
        .map_or(Reason::InvalidValue, |problem| problem.reason);
    let mut combined = Failure::new(reason, format!("{headline} {} problems:", problems.len()));
    for problem in problems {
        if problem.issues.is_empty() {
            combined = combined.issue(problem.headline.clone());
        } else {
            for issue in &problem.issues {
                combined = combined.issue(issue.clone());
            }
        }
    }
    combined
}
