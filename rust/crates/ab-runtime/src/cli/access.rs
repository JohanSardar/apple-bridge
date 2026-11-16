use super::{
    Failure, Outcome,
    failure::{Reason, Step},
    modes::{self, Modes},
    tree::{AgentCreateArgs, DecisionArgs, ListArgs},
    ui,
};
use crate::product_cli;
use ab_trust::{AgentLevel, Authority, Grant, Principal};
use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static LEGACY_REVOKE: AtomicBool = AtomicBool::new(false);

pub fn typed_as(words: &[String]) {
    let first = words.first().map(String::as_str);
    let second = words.get(1).map(String::as_str);
    let legacy = matches!(first, Some("revoke" | "scope"))
        || (matches!(first, Some("agent" | "agents")) && second == Some("level"))
        || (first == Some("agent") && second == Some("revoke"))
        || (first == Some("web") && second == Some("lock"));
    LEGACY_REVOKE.store(legacy, Ordering::Relaxed);
}

fn legacy_grace(what: &str) -> bool {
    let modes = Modes::get();
    if !modes.yes && !modes.interactive() && LEGACY_REVOKE.load(Ordering::Relaxed) {
        ui::warn(
            "deprecated",
            &format!(
                "{what} without --yes outside a terminal is deprecated. Pass --yes; the next major version will refuse."
            ),
        );
        return true;
    }
    false
}

fn confirm_unless_legacy(what: &str, consent: &ui::Consent<'_>) -> Outcome {
    if legacy_grace(what) {
        if Modes::get().json {
            return Ok(());
        }
        for (label, value) in &consent.rows {
            ui::row(label, value);
        }
        return Ok(());
    }
    ui::confirm(consent)
}

fn expiry(text: Option<&str>) -> Outcome<Option<jiff::Timestamp>> {
    let Some(text) = text else {
        return Ok(None);
    };
    let length = super::commands::duration("--expires-in", text)?;
    let seconds = i64::try_from(length.as_secs())
        .map_err(|_| Failure::invalid("Can't use that --expires-in. It's too long."))?;
    jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(seconds))
        .map(Some)
        .map_err(|_| Failure::invalid("Can't use that --expires-in. It's too long."))
}

fn expires_text(expires_at: Option<&jiff::Timestamp>) -> String {
    expires_at.map_or_else(
        || "never".to_owned(),
        |when| ui::local_time(&when.to_string()),
    )
}

const LIST_DEFAULT: usize = 50;

struct Page {
    list: &'static str,
    limit: usize,
    after: Option<String>,
    bounded: bool,
}

fn page(list: &'static str, args: &ListArgs, home: &Path) -> Outcome<Page> {
    let given = args.limit.as_deref().map(super::read::limit).transpose()?;
    let mut page = Page {
        list,
        limit: given.unwrap_or(LIST_DEFAULT),
        after: None,
        bounded: given.is_some() || args.next.is_some(),
    };
    let Some(text) = &args.next else {
        return Ok(page);
    };
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            "Can't use that --next cursor. Copy it whole from the previous page.",
        )
    };
    let bytes = BASE64URL_NOPAD
        .decode(text.trim().as_bytes())
        .map_err(|_| invalid())?;
    let cursor: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if cursor["list"] != list {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!("Can't use that --next cursor here. It doesn't continue bridge {list} list."),
        ));
    }
    if cursor["home"] != super::read::home_tag(home) {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't use that --next cursor here. It belongs to another home.",
        )
        .sentence("Pass the same --home as the list that printed it."));
    }
    page.after = Some(cursor["after"].as_str().ok_or_else(invalid)?.to_owned());
    if given.is_none() {
        page.limit = cursor["limit"]
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(LIST_DEFAULT);
    }
    Ok(page)
}

struct Sliced<T> {
    items: Vec<T>,
    total: usize,
    next: Option<String>,
}

fn slice<T>(
    items: Vec<T>,
    id: impl Fn(&T) -> String,
    page: &Page,
    home: &Path,
) -> Outcome<Sliced<T>> {
    let total = items.len();
    let start = match &page.after {
        Some(after) => {
            items
                .iter()
                .position(|item| id(item) == *after)
                .ok_or_else(|| {
                    Failure::new(
                        Reason::InvalidValue,
                        "Can't continue that list. The entry its --next cursor points at is gone."
                            .to_owned(),
                    )
                    .then(
                        "List from the start",
                        std::iter::once("bridge")
                            .chain(page.list.split(' '))
                            .chain(["list"]),
                    )
                })?
                + 1
        }
        None => 0,
    };
    let rest: Vec<T> = items.into_iter().skip(start).collect();
    let more = rest.len() > page.limit;
    let items: Vec<T> = rest.into_iter().take(page.limit).collect();
    let next = if more {
        items.last().map(|last| {
            BASE64URL_NOPAD.encode(
                json!({"v": 1, "list": page.list, "after": id(last), "limit": page.limit, "home": super::read::home_tag(home)})
                    .to_string()
                    .as_bytes(),
            )
        })
    } else {
        None
    };
    Ok(Sliced { items, total, next })
}

fn more_note<T>(sliced: &Sliced<T>, argv: &[&str]) {
    let Some(next) = &sliced.next else {
        return;
    };
    ui::note(&format!(
        "Showing {} of {}.",
        sliced.items.len(),
        sliced.total
    ));
    let mut command: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
    command.extend(["--next".to_owned(), next.clone()]);
    ui::next_steps(&[Step::new("See the next page", command)]);
}

fn set_up(home: &Path) -> Outcome<Authority> {
    if !home.is_dir() {
        return Err(Failure::missing_home(home));
    }
    open(home)
}

fn widens(before: &[Grant], after: &[Grant]) -> bool {
    after
        .iter()
        .any(|grant| !before.iter().any(|old| old.as_str() == grant.as_str()))
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
                ui::clean_input(text),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<&'a Value>,
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
        expires_at: present("expires_at"),
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
            "Piped {command} prints JSON until the next major version, then a table. Pass --json to keep JSON."
        ),
    );
}

pub fn agents_list(home: &Path, args: &ListArgs) -> Outcome {
    let paging = page("agents", args, home)?;
    let authority = set_up(home)?;
    let agents = authority.agents()?;
    let modes = Modes::get();
    let legacy = modes.legacy_json() || modes.json && modes::legacy_spelling();
    let sliced = if legacy && !paging.bounded {
        let total = agents.len();
        Sliced {
            items: agents,
            total,
            next: None,
        }
    } else {
        slice(agents, |agent| agent.id.clone(), &paging, home)?
    };
    let documents: Vec<AgentSummary<'_>> = sliced.items.iter().map(summary).collect();
    if legacy {
        if modes.json {
            ui::json_document(serde_json::to_value(&documents)?);
        } else {
            legacy_notice("bridge agents list");
            ui::exact(&documents);
        }
        return Ok(());
    }
    if modes.json {
        ui::json_keyed(
            "agents",
            json!({"agents": documents, "next_cursor": sliced.next}),
        );
        return Ok(());
    }
    if sliced.items.is_empty() {
        ui::note("No agents on this Mac.");
        ui::next_steps(&[Step::new(
            "Create one",
            [
                "bridge",
                "agents",
                "create",
                "--name",
                "<NAME>",
                "--level",
                "read",
                "--expires-in",
                "30d",
            ],
        )]);
        if !modes.stdout_table() {
            ui::table(
                &["ID", "Name", "Level", "Access", "Created", "Revoked"],
                &[],
            );
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sliced
        .items
        .iter()
        .map(|agent| {
            let shown = summary(agent);
            let when = |value: &str| {
                if modes.stdout_table() {
                    ui::age_short(value)
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
    more_note(&sliced, &["bridge", "agents", "list"]);
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
        ("Created", ui::local_time(shown.created_at)),
        (
            "Expires",
            shown
                .expires_at
                .and_then(Value::as_str)
                .filter(|when| !when.starts_with("0001"))
                .map_or_else(|| "never".to_owned(), ui::local_time),
        ),
        (
            "Revoked",
            shown
                .revoked_at
                .and_then(Value::as_str)
                .map_or_else(|| "no".to_owned(), ui::local_time),
        ),
        (
            "Last used",
            shown.last_used.map_or_else(
                || "never".to_owned(),
                |value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), ui::local_time)
                },
            ),
        ),
    ];
    for (label, value) in lines {
        ui::outln(&format!("{label:<16}{}", ui::clean(&value)));
    }
    Ok(())
}

fn lasting_consent(owner: &str, what: &str, rows: &[(&str, String)]) -> Outcome<bool> {
    let modes = Modes::get();
    if modes.yes {
        return Ok(false);
    }
    if !modes.interactive() {
        ui::warn(
            "never_expires",
            &format!("{owner} never expires. Pass --expires-in to limit it."),
        );
        return Ok(false);
    }
    for (label, value) in rows {
        ui::row(label, value);
    }
    if ui::yes_or_no(&format!("Create {what} with access that never expires?"))? {
        Ok(true)
    } else {
        Err(Failure::canceled(
            "Canceled. Nothing was created. Pass --expires-in, such as --expires-in 30d, to limit it.",
        ))
    }
}

#[allow(clippy::too_many_lines)]
pub fn agents_create(home: &Path, args: &AgentCreateArgs) -> Outcome {
    let name = args.name.trim();
    let mut problems = Vec::new();
    if name.is_empty() || name.len() > 120 {
        problems.push(Failure::new(
            Reason::InvalidValue,
            "Can't use that --name. Names are 1 to 120 bytes; accented letters and emoji take 2 to 4 each.",
        ));
    } else if name.contains('@') && name.contains(':') {
        problems.push(Failure::new(
            Reason::InvalidValue,
            "Can't use that --name. Names can't include a user name and password, such as user:pass@.",
        ));
    }
    let level = if args.level.is_empty() {
        Ok(AgentLevel::DEFAULT)
    } else {
        level(&args.level, false)
    };
    let grants = parse_grants(&args.scope, true);
    let expires = expiry(args.expires_in.as_deref());
    let (level, grants, expires) = match (level, grants, expires) {
        (Ok(level), Ok(grants), Ok(expires)) if problems.is_empty() => (level, grants, expires),
        (level, grants, expires) => {
            problems.extend(level.err());
            problems.extend(grants.err());
            problems.extend(expires.err());
            return Err(combine("Can't create that agent.", problems));
        }
    };
    let modes = Modes::get();
    let tailnet = if args.tailnet {
        Some(tailnet_mcp(home)?)
    } else {
        None
    };
    let authority = open(home)?;
    let asked = if expires.is_none() {
        lasting_consent(
            &format!("{}'s token", ui::clean(name)),
            &format!("agent {}", ui::clean(name)),
            &[
                ("Name", ui::clean(name)),
                ("Level", level.as_str().to_owned()),
                ("Access", access_text(&grants)),
                ("Expires", "never".to_owned()),
            ],
        )?
    } else {
        false
    };
    let (agent, token) = authority
        .create_agent_until(name, level, grants, expires)
        .map_err(|error| match error {
            ab_trust::Error::Unauthorized => Failure::new(
                Reason::PermissionDenied,
                "Can't create that agent. This home can only grant the access it was given.",
            ),
            other => Failure::from(other),
        })?;
    let agent = match &tailnet {
        Some(resource) => authority
            .bind_to_resource(&agent.id, resource)
            .map_err(Failure::from)?,
        None => agent,
    };
    ui::mask_secret(&token);
    if modes.json || modes.legacy_json() {
        #[derive(serde::Serialize)]
        struct Created<'a> {
            agent: AgentSummary<'a>,
            token: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            address: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            outcome: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            next: Option<Vec<Value>>,
        }
        let created = Created {
            agent: summary(&agent),
            token: &token,
            address: tailnet.as_deref(),
            outcome: modes.json.then_some("created"),
            next: modes.json.then(|| {
                vec![
                    Step::new(
                        "Check it",
                        ["bridge", "agents", "inspect", &agent.id, "--json"],
                    )
                    .to_json(),
                ]
            }),
        };
        if modes.json {
            ui::json_keyed("agent", serde_json::to_value(&created)?);
        } else {
            legacy_notice("bridge agents create");
            ui::exact(&created);
        }
        return Ok(());
    }
    ui::outln(&token);
    ui::receipt(
        "Created",
        &agent.id,
        &format!("Created agent {} ({}).", ui::clean(name), agent.id),
    );
    if !asked {
        agent_rows(&agent);
        ui::row("Expires", &expires_text(expires.as_ref()));
    }
    if let Some(resource) = &tailnet {
        ui::row("Address", resource);
        ui::row("Header", "Authorization: Bearer <the token above>");
    }
    ui::note("The token above is shown once. Keep it secret.");
    Ok(())
}

fn tailnet_mcp(home: &Path) -> Outcome<String> {
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't read the Tailscale state.")
            .remote("Remote", error.to_string())
    })?;
    status
        .routes
        .get("tailnet")
        .filter(|route| route.state == "on" && route.url.starts_with("https://"))
        .map(|route| format!("{}/mcp", route.url))
        .ok_or_else(|| {
            Failure::new(
                Reason::Conflict,
                "Can't make a Tailscale token. The Tailscale route isn't on.",
            )
            .then("Turn it on", ["bridge", "tailscale"])
        })
}

#[allow(clippy::too_many_lines)]
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
    let chosen = new_level.map(|text| level(text, true)).transpose();
    let grants = scope.map(|scope| parse_grants(scope, true)).transpose();
    let (chosen, grants) = match (chosen, grants) {
        (Ok(chosen), Ok(grants)) => (chosen, grants),
        (chosen, grants) => {
            let mut problems = Vec::new();
            problems.extend(chosen.err());
            problems.extend(grants.err());
            return Err(combine("Can't update that agent.", problems));
        }
    };
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
    let writes: Vec<&str> = grants
        .iter()
        .flatten()
        .map(Grant::as_str)
        .filter(|grant| grant.ends_with(":write"))
        .collect();
    if before.level == AgentLevel::Read.as_str() && !writes.is_empty() {
        return Err(Failure::new(
            Reason::ReadOnly,
            format!("Can't give {id} {}. It's a read agent.", writes.join(",")),
        )
        .sentence(
            "A read agent can only read. Revoke it and create a new agent to give it write access.",
        )
        .then(
            "Create a new agent",
            [
                "bridge",
                "agents",
                "create",
                "--name",
                "<NAME>",
                "--level",
                "full",
                "--scope",
                &writes.join(","),
            ],
        ));
    }
    let unchanged_level = chosen.is_none_or(|level| level.as_str() == before.level);
    let unchanged_grants = grants
        .as_ref()
        .is_none_or(|grants| access_text(grants) == access_text(&before.grants));
    let raises_level = chosen.is_some_and(|level| {
        level == AgentLevel::Full && before.level != AgentLevel::Full.as_str()
    });
    let widens_access = grants
        .as_ref()
        .is_some_and(|grants| widens(&before.grants, grants));
    if raises_level || widens_access {
        let name = before
            .extra
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut rows = vec![("Agent".to_owned(), id.to_owned())];
        if !name.is_empty() {
            rows.push(("Name".to_owned(), ui::clean(&name)));
        }
        if let Some(level) = chosen.filter(|_| !unchanged_level) {
            rows.push((
                "Level".to_owned(),
                format!("{} → {}", before.level, level.as_str()),
            ));
        }
        if let Some(grants) = grants.as_ref().filter(|_| !unchanged_grants) {
            rows.push((
                "Access".to_owned(),
                format!("{} → {}", access_text(&before.grants), access_text(grants)),
            ));
        }
        let consequence = if raises_level {
            "At level full the agent changes your data at once, without asking you first."
        } else {
            "The agent can read or change the added sources as soon as this is saved."
        };
        let mut argv = vec![
            "bridge".to_owned(),
            "agents".to_owned(),
            "update".to_owned(),
            id.to_owned(),
        ];
        if let Some(level) = new_level {
            argv.extend(["--level".to_owned(), level.to_owned()]);
        }
        if let Some(scope) = scope {
            argv.extend(["--scope".to_owned(), scope.to_owned()]);
        }
        argv.push("--yes".to_owned());
        confirm_unless_legacy(
            "Widening an agent's access",
            &ui::Consent {
                action: format!("widen the access of {id}"),
                rows,
                consequence: consequence.to_owned(),
                question: format!("Change {id}?"),
                with_yes: Step::new("Change it", argv),
                nothing_changed: "Nothing was changed.",
            },
        )?;
    }
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
        ui::json_document(json!({
            "agent": summary_json(after),
            "previous": {"level": before.level, "scopes": before.grants},
            "outcome": outcome,
            "next": [Step::new("Check it", ["bridge", "agents", "inspect", &after.id, "--json"]).to_json()],
        }));
        return Ok(());
    }
    if modes.legacy_json() {
        legacy_notice("bridge agents update");
        ui::exact(&summary(after));
        return Ok(());
    }
    if unchanged {
        ui::receipt(
            "Already set",
            &after.id,
            &format!("Agent {} already has that access.", after.id),
        );
    } else {
        ui::receipt(
            "Updated",
            &after.id,
            &format!("Updated agent {}.", after.id),
        );
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

pub fn tokens_create(home: &Path, scope: &str, remote: bool, expires_in: Option<&str>) -> Outcome {
    let grants = parse_grants(scope, false);
    let expires = expiry(expires_in);
    let (grants, expires) = match (grants, expires) {
        (Ok(grants), Ok(expires)) => (grants, expires),
        (grants, expires) => {
            let mut problems = Vec::new();
            problems.extend(grants.err());
            problems.extend(expires.err());
            return Err(combine("Can't create that token.", problems));
        }
    };
    let modes = Modes::get();
    let authority = open(home)?;
    let asked = if expires.is_none() {
        lasting_consent(
            "This token",
            "a token",
            &[
                ("Access", access_text(&grants)),
                ("Remote", if remote { "allowed" } else { "no" }.to_owned()),
                ("Expires", "never".to_owned()),
            ],
        )?
    } else {
        false
    };
    let token = authority
        .create_token_until(grants.clone(), remote, expires)
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
    let scopes: Vec<&str> = grants.iter().map(Grant::as_str).collect();
    if modes.json && modes::legacy_spelling() {
        ui::json_document(json!({
            "token": token,
            "id": id,
            "scopes": scopes,
            "remote": remote,
            "expires_at": expires.map(|when| when.to_string()),
            "outcome": "created",
        }));
        return Ok(());
    }
    if modes.json {
        ui::json_keyed(
            "token",
            json!({
                "token": {
                    "id": id,
                    "scopes": scopes,
                    "remote": remote,
                    "expiresAt": expires.map(|when| when.to_string()),
                },
                "secret": token,
                "outcome": "created",
                "next": [Step::new("Revoke it when you're done", ["bridge", "tokens", "revoke", &id]).to_json()],
            }),
        );
        return Ok(());
    }
    ui::outln(&token);
    if modes.stdout_tty() {
        ui::receipt("Created", &id, &format!("Created token {id}."));
        if !asked {
            ui::row("Access", &access_text(&grants));
            if remote {
                ui::row("Remote", "allowed");
            }
            ui::row("Expires", &expires_text(expires.as_ref()));
        }
        ui::note("The token above is shown once. Keep it secret.");
    }
    Ok(())
}

fn principal_json(principal: &Principal) -> Value {
    if principal.kind == "agent" {
        return summary_json(principal);
    }
    let present = |key: &str| {
        principal
            .extra
            .get(key)
            .filter(|value| !value.is_null())
            .cloned()
    };
    let mut value = json!({
        "id": principal.id,
        "kind": principal.kind,
        "scopes": principal.grants,
    });
    if principal.kind == "token" {
        value["remote"] = json!(principal.remote);
        value["expiresAt"] = present("expires_at")
            .filter(|when| when.as_str().is_none_or(|when| !when.starts_with("0001")))
            .unwrap_or(Value::Null);
    }
    if let Some(revoked) = present("revoked_at") {
        value["revokedAt"] = revoked;
    }
    value
}

fn label_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
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
    let list = ["bridge", "agents", "list", "--json"];
    if !principal.active() {
        if Modes::get().json && modes::legacy_spelling() {
            ui::json_document(json!({"id": id, "kind": noun, "outcome": "unchanged"}));
        } else if Modes::get().json {
            ui::json_keyed(
                noun,
                json!({noun: principal_json(&principal), "outcome": "unchanged", "next": [Step::new("Check it", list).to_json()]}),
            );
        } else {
            ui::receipt(
                "Already revoked",
                id,
                &format!("{} {id} is already revoked.", label_case(noun)),
            );
        }
        return Ok(());
    }
    let shown = if name.is_empty() {
        format!("{noun} {id}")
    } else {
        format!("{noun} {}", ui::clean(&name))
    };
    let mut rows = vec![(label_case(noun), id.to_owned())];
    if !name.is_empty() {
        rows.push(("Name".to_owned(), ui::clean(&name)));
    }
    rows.push(("Access lost".to_owned(), access_text(&principal.grants)));
    let modes = Modes::get();
    if !legacy_grace("Revoking") {
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
        ui::confirm(&ui::Consent {
            action: format!("revoke {noun} {id}"),
            rows,
            consequence: "Anything using its token stops working at once. This cannot be undone."
                .to_owned(),
            question: format!("Revoke {shown}?"),
            with_yes: Step::new("Revoke it", argv),
            nothing_changed: "Nothing was revoked.",
        })?;
    }
    authority.revoke(id).map_err(|_| not_found(id))?;
    if modes.json && modes::legacy_spelling() {
        ui::json_document(json!({
            "id": id,
            "kind": noun,
            "outcome": "revoked",
            "next": [Step::new("Check it", list).to_json()],
        }));
        return Ok(());
    }
    if modes.json {
        let after = credential(&authority, id)?.unwrap_or(principal);
        ui::json_keyed(
            noun,
            json!({noun: principal_json(&after), "outcome": "revoked", "next": [Step::new("Check it", list).to_json()]}),
        );
        return Ok(());
    }
    let sentence = if name.is_empty() {
        format!("Revoked {shown}.")
    } else {
        format!("Revoked {shown} ({id}).")
    };
    ui::receipt("Revoked", id, &sentence);
    if !name.is_empty() {
        ui::row("Name", &name);
    }
    ui::row("Access", &access_text(&principal.grants));
    Ok(())
}

#[allow(clippy::too_many_lines)]
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
    let noun = match before.kind.as_str() {
        "agent" => "agent",
        "token" => "token",
        "replica" => "replica",
        _ => "credential",
    };
    if access_text(&before.grants) == access_text(&grants) {
        if Modes::get().json && modes::legacy_spelling() {
            ui::json_document(json!({"id": id, "scopes": before.grants, "outcome": "unchanged"}));
        } else if Modes::get().json {
            ui::json_keyed(
                noun,
                json!({noun: principal_json(&before), "outcome": "unchanged", "next": [Step::new("Check it", ["bridge", "agents", "list", "--json"]).to_json()]}),
            );
        } else {
            ui::receipt(
                "Already set",
                id,
                &format!("{} {id} already has that access.", label_case(noun)),
            );
            ui::row("Access", &access_text(&before.grants));
        }
        return Ok(());
    }
    if widens(&before.grants, &grants) {
        let mut argv = vec![
            "bridge".to_owned(),
            "tokens".to_owned(),
            "update".to_owned(),
            id.to_owned(),
            "--scope".to_owned(),
            scope.to_owned(),
            "--yes".to_owned(),
        ];
        if LEGACY_REVOKE.load(Ordering::Relaxed) {
            argv.drain(1..3);
            argv.insert(1, "scope".to_owned());
        }
        confirm_unless_legacy(
            "Widening access",
            &ui::Consent {
                action: format!("widen the access of {id}"),
                rows: vec![
                    (label_case(&before.kind), id.to_owned()),
                    (
                        "Access".to_owned(),
                        format!("{} → {}", access_text(&before.grants), access_text(&grants)),
                    ),
                ],
                consequence: "Anything using this credential can use the added sources as soon as this is saved."
                    .to_owned(),
                question: format!("Change {id}?"),
                with_yes: Step::new("Change it", argv),
                nothing_changed: "Nothing was changed.",
            },
        )?;
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
    if Modes::get().json && modes::legacy_spelling() {
        ui::json_document(json!({
            "id": id,
            "scopes": after.grants,
            "previous": before.grants,
            "outcome": "updated",
            "next": [Step::new("Check it", ["bridge", "agents", "list", "--json"]).to_json()],
        }));
        return Ok(());
    }
    if Modes::get().json {
        ui::json_keyed(
            noun,
            json!({
                noun: principal_json(&after),
                "previous": {"scopes": before.grants},
                "outcome": "updated",
                "next": [Step::new("Check it", ["bridge", "agents", "list", "--json"]).to_json()],
            }),
        );
        return Ok(());
    }
    ui::receipt("Updated", id, &format!("Updated {noun} {id}."));
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

pub async fn connectors_list(home: &Path, args: &ListArgs) -> Outcome {
    let paging = page("connectors", args, home)?;
    if !home.is_dir() {
        return Err(Failure::missing_home(home));
    }
    let pending = crate::connector::command(home, "pending", "", "")
        .await
        .map_err(|_| connector_unreachable(home))?;
    let modes = Modes::get();
    let legacy = modes.legacy_json() || modes.json && modes::legacy_spelling();
    if legacy && !paging.bounded {
        if modes.json {
            ui::json_document(pending);
        } else {
            legacy_notice("bridge connectors list");
            ui::exact_json(&pending);
        }
        return Ok(());
    }
    let mut items = pending.as_array().cloned().unwrap_or_default();
    items.sort_by(|a, b| {
        a["expires_at"]
            .as_str()
            .cmp(&b["expires_at"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    let sliced = slice(
        items,
        |item| item["id"].as_str().unwrap_or_default().to_owned(),
        &paging,
        home,
    )?;
    if legacy {
        ui::json_document(Value::Array(sliced.items));
        return Ok(());
    }
    if modes.json {
        ui::json_keyed(
            "connectors",
            json!({"connectors": sliced.items, "next_cursor": sliced.next}),
        );
        return Ok(());
    }
    if sliced.items.is_empty() {
        ui::note("No apps are waiting to connect.");
        if !modes.stdout_table() {
            ui::table(&["Request", "App", "Redirect", "Access", "Expires"], &[]);
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sliced
        .items
        .iter()
        .map(|item| {
            let text = |key: &str| item[key].as_str().unwrap_or_default().to_owned();
            vec![
                text("id"),
                text("client_name"),
                text("redirect_host"),
                text("access"),
                if modes.stdout_table() {
                    ui::local_time(&text("expires_at"))
                } else {
                    text("expires_at")
                },
            ]
        })
        .collect();
    ui::table(&["Request", "App", "Redirect", "Access", "Expires"], &rows);
    if let Some(first) = sliced.items.first().and_then(|item| item["id"].as_str()) {
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
    more_note(&sliced, &["bridge", "connectors", "list"]);
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub async fn connectors_decide(home: &Path, decision: &DecisionArgs, allow: bool) -> Outcome {
    let action = if allow { "allow" } else { "deny" };
    let missing: Vec<&str> = [
        ("--request", decision.request.is_none()),
        ("--code", decision.code.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect();
    if !missing.is_empty() {
        let headline = if missing.len() == 1 {
            format!("Can't {action} a connector without {}.", missing[0])
        } else {
            format!(
                "Can't {action} a connector. {} inputs are missing: {}.",
                missing.len(),
                missing.join(", ")
            )
        };
        return Err(Failure::new(Reason::MissingArgument, headline)
            .data("missing", missing.clone())
            .then(
                "See the apps waiting and their request IDs",
                ["bridge", "connectors", "list"],
            )
            .then(
                format!("Then {action} one"),
                [
                    "bridge",
                    "connectors",
                    action,
                    "--request",
                    "<REQUEST>",
                    "--code",
                    "<CODE>",
                ],
            ));
    }
    let request_id = decision.request.as_deref().unwrap_or_default();
    let code = decision.code.as_deref().unwrap_or_default();
    let valid_request = !request_id.is_empty()
        && request_id.len() <= 128
        && request_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-:".contains(c));
    if !valid_request {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --request {}. Request IDs come from bridge connectors list.",
                ui::clean(request_id)
            ),
        ));
    }
    if code.is_empty() || code.len() > 16 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't use that --code. Copy the code shown on the app's page.",
        ));
    }
    let pending = crate::connector::command(home, "pending", "", "")
        .await
        .map_err(|_| connector_unreachable(home))?;
    let request = pending
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == request_id).cloned());
    let result = crate::connector::command(home, action, request_id, code)
        .await
        .map_err(|_| {
            if request.is_none() {
                Failure::new(
                    Reason::NotFound,
                    format!("Couldn't find request {}. It may have expired.", ui::clean(request_id)),
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
            map.insert(
                "next".into(),
                json!([
                    Step::new("Check it", ["bridge", "connectors", "list", "--json"]).to_json()
                ]),
            );
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
    ui::receipt(
        if allow { "Allowed" } else { "Denied" },
        &app,
        &format!(
            "{} {}.",
            if allow { "Allowed" } else { "Denied" },
            ui::clean(&app)
        ),
    );
    ui::row("Request", request_id);
    if allow && let Some(access) = request.as_ref().and_then(|item| item["access"].as_str()) {
        ui::row("Access", access);
    }
    Ok(())
}

fn remote_url(home: &Path) -> Outcome<String> {
    let off = || {
        Failure::new(
            Reason::Conflict,
            "Can't use the Tailscale address. Tailscale access is off.",
        )
        .then("Turn it on", ["bridge", "tailscale"])
    };
    let status: Value = ab_remote::state::read(home, "status").map_err(|_| off())?;
    if status["state"] != "on" {
        return Err(off());
    }
    status["url"].as_str().map(str::to_owned).ok_or_else(off)
}

fn local_host(home: &Path) -> Outcome<String> {
    let address = super::commands::load_config(home)?
        .map(|config| config.api_address)
        .filter(|address| !address.is_empty())
        .unwrap_or_else(|| "127.0.0.1:7780".to_owned());
    let port = address
        .rsplit_once(':')
        .map_or("7780", |(_, port)| port)
        .to_owned();
    Ok(format!("http://localhost:{port}"))
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

pub fn web_enroll(home: &Path, host: Option<&str>, public: bool) -> Outcome {
    if public && host.is_some() {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --host with --public. --public uses the remote address.",
        ));
    }
    let host = if public {
        let url = remote_url(home)?;
        if !url.starts_with("https://") {
            return Err(Failure::new(
                Reason::Conflict,
                "Can't use the Tailscale address. It isn't HTTPS yet.",
            )
            .then("Check it", ["bridge", "status"]));
        }
        url
    } else if let Some(host) = host {
        host.to_owned()
    } else {
        local_host(home)?
    };
    if !valid_web_host(&host) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --host {}. Use https://HOST, or http://localhost:PORT on this Mac.",
                ui::clean_input(&host)
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

pub fn sessions_list(home: &Path, args: &ListArgs) -> Outcome {
    let paging = page("web sessions", args, home)?;
    let mut sessions = set_up(home)?.device_sessions()?;
    sessions.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });
    let modes = Modes::get();
    let legacy = modes.legacy_json() || modes.json && modes::legacy_spelling();
    if legacy && !paging.bounded {
        if modes.json {
            ui::json_document(serde_json::to_value(&sessions)?);
        } else {
            legacy_notice("bridge web sessions list");
            ui::exact_json(&serde_json::to_value(&sessions)?);
        }
        return Ok(());
    }
    let sliced = slice(sessions, |session| session.device_id.clone(), &paging, home)?;
    if legacy {
        ui::json_document(serde_json::to_value(&sliced.items)?);
        return Ok(());
    }
    if modes.json {
        let shown: Vec<Value> = sliced
            .items
            .iter()
            .map(|session| {
                json!({
                    "device_id": session.device_id,
                    "kind": session.kind,
                    "created_at": session.created_at,
                    "last_seen": session.last_seen,
                    "expires_at": session.expires_at,
                })
            })
            .collect();
        ui::json_keyed(
            "sessions",
            json!({"sessions": shown, "next_cursor": sliced.next}),
        );
        return Ok(());
    }
    if sliced.items.is_empty() {
        ui::note("No browsers are signed in.");
        if !modes.stdout_table() {
            ui::table(
                &["Device", "Kind", "Signed in", "Last seen", "Expires"],
                &[],
            );
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = sliced
        .items
        .iter()
        .map(|session| {
            let when = |value: &str| {
                if modes.stdout_table() {
                    ui::age_short(value)
                } else {
                    value.to_owned()
                }
            };
            vec![
                session.device_id.clone(),
                session.kind.clone(),
                when(&session.created_at),
                when(&session.last_seen),
                if modes.stdout_table() {
                    ui::local_time(&session.expires_at)
                } else {
                    session.expires_at.clone()
                },
            ]
        })
        .collect();
    ui::table(
        &["Device", "Kind", "Signed in", "Last seen", "Expires"],
        &rows,
    );
    more_note(&sliced, &["bridge", "web", "sessions", "list"]);
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
    confirm_unless_legacy(
        "Signing out browsers",
        &ui::Consent {
            action: format!("sign out {count} {plural}"),
            rows: vec![
                ("Browsers".to_owned(), count.to_string()),
                ("Passkeys".to_owned(), format!("{devices} enrolled")),
            ],
            consequence: "Each browser is signed out of this Mac's pages and needs a new passkey enrollment to sign in again.".to_owned(),
            question: format!("Sign out {count} {plural}?"),
            with_yes: Step::new(
                "Sign them out",
                ["bridge", "web", "sessions", "revoke", "--all", "--yes"],
            ),
            nothing_changed: "Nobody was signed out.",
        },
    )?;
    authority.lock_web()?;
    if Modes::get().json {
        ui::json_document(json!({
            "revoked": count,
            "outcome": "revoked",
            "next": [Step::new("Check it", ["bridge", "web", "sessions", "list", "--json"]).to_json()],
        }));
    } else {
        ui::receipt(
            "Signed out",
            &format!("{count} {plural}"),
            &format!("Signed out {count} {plural}."),
        );
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn already_signed_out() -> Outcome {
    if Modes::get().json {
        ui::json_document(json!({"revoked": 0, "outcome": "unchanged"}));
    } else {
        ui::receipt(
            "Already signed out",
            "No browser sessions were open.",
            "No browsers are signed in.",
        );
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
