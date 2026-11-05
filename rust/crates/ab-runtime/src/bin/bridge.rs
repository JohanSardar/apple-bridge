#![forbid(unsafe_code)]
use ab_runtime::{Config, Error, default_home};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
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
const KNOWN_FLAGS: &[&str] = &[
    "home",
    "host",
    "public",
    "helper-app",
    "helper-lab-commands",
    "via",
    "name",
    "level",
    "expires-in",
    "request",
    "code",
    "dial-replica",
    "iphone",
    "via-remote",
    "backup",
    "accept-ledger-loss",
    "role",
    "listen",
    "api-addr",
    "peer-addr",
    "socket",
    "scope",
    "remote",
    "blob-cap",
    "blob-grace-ms",
    "retention-rows",
    "retention-age",
    "pack",
    "no-ui",
    "force",
    "snippets",
    "private",
    "domain",
    "relay-url",
];
#[derive(Debug)]
struct Arguments {
    verb: String,
    value: String,
    action: String,
    flags: HashMap<String, String>,
}
impl Arguments {
    fn parse() -> Result<Self, Error> {
        let mut args = std::env::args().skip(1);
        let verb = args.next().ok_or(Error::Invalid)?;
        if verb == "token" && args.next().as_deref() != Some("create") {
            return Err(Error::Invalid);
        }
        let action = if [
            "agent",
            "remote",
            "web",
            "connector",
            "shortcuts",
            "permissions",
            "selftest",
        ]
        .contains(&verb.as_str())
        {
            args.next().ok_or(Error::Invalid)?
        } else {
            String::new()
        };
        let mut flags = HashMap::new();
        let mut value = String::new();
        while let Some(argument) = args.next() {
            if let Some(flag) = argument
                .strip_prefix("--")
                .or_else(|| argument.strip_prefix('-'))
            {
                let (key, content) = if let Some((key, content)) = flag.split_once('=') {
                    (key.to_owned(), content.to_owned())
                } else if BOOLEAN_FLAGS.contains(&flag) {
                    (flag.to_owned(), "true".into())
                } else {
                    (flag.to_owned(), args.next().ok_or(Error::Invalid)?)
                };
                let lab_flag =
                    cfg!(feature = "lab-harness") && ["id", "fingerprint"].contains(&key.as_str());
                if !lab_flag && !KNOWN_FLAGS.contains(&key.as_str()) {
                    return Err(Error::Invalid);
                }
                flags.insert(key, content);
            } else if value.is_empty() {
                value = argument;
            } else {
                return Err(Error::Invalid);
            }
        }
        Ok(Self {
            verb,
            value,
            action,
            flags,
        })
    }
    fn config(&self, home: PathBuf) -> Result<Config, Error> {
        Ok(Config {
            role: self.flag("role", "gateway"),
            home,
            api_address: self.flag("api-addr", "127.0.0.1:7780"),
            peer_address: self.flag("peer-addr", ""),
            listen: self.flag("listen", ""),
            socket: self.flag("socket", ""),
            helper_app: ab_runtime::bundle::helper_app(
                &self.flag("helper-app", ""),
                std::env::current_exe().ok().as_deref(),
            ),
            helper_lab_commands: self.flag("helper-lab-commands", "false") == "true",
            retention_rows: self
                .flag("retention-rows", "0")
                .parse()
                .map_err(|_| Error::Invalid)?,
            retention_age: ab_runtime::duration::parse(&self.flag("retention-age", "0"))?,
            blob_cap: self
                .flag("blob-cap", "0")
                .parse()
                .map_err(|_| Error::Invalid)?,
            blob_grace_ms: self
                .flag("blob-grace-ms", "0")
                .parse()
                .map_err(|_| Error::Invalid)?,
        })
    }
    fn flag(&self, key: &str, default: &str) -> String {
        self.flags
            .get(key)
            .map_or_else(|| default.into(), Clone::clone)
    }
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn product_verb(args: &Arguments) -> Result<bool, Error> {
    use ab_runtime::product_cli;
    match args.verb.as_str() {
        "permissions" if args.action == "request" => {
            if args.value.is_empty() || args.flags.keys().any(|key| key != "helper-app") {
                return Err(Error::Invalid);
            }
            product_cli::permissions_request(
                std::path::Path::new("open"),
                &product_cli::default_report_dir(),
                &args.flag("helper-app", ""),
                &args.value,
                None,
            )?;
        }
        "selftest" | "setup" => {
            if args.verb == "selftest" && args.action != "messages" {
                return Err(Error::Invalid);
            }
            let mut app = args.flag("helper-app", "");
            if app.is_empty()
                && let Some(bundle) = product_cli::detect_helper_app()
            {
                app = bundle.to_string_lossy().into_owned();
            }
            let result = product_cli::messages_selftest(
                std::path::Path::new("open"),
                &product_cli::default_report_dir(),
                &app,
                args.flag("force", "false") == "true",
                args.flag("no-ui", "false") != "true",
                None,
            )?;
            println!("{}", result.data);
            if !result.proven() {
                eprintln!(
                    "messages self-test {}; failed operations remain unproven",
                    result.state
                );
                std::process::exit(1);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
fn offline(args: &Arguments, home: &std::path::Path) -> Result<bool, Error> {
    #[cfg(feature = "lab-harness")]
    if args.verb == "lab-operation-catalog" {
        if !args.value.is_empty() || args.flags.keys().any(|key| key != "home") {
            return Err(Error::Invalid);
        }
        let mut descriptors = ab_catalog::operations()?;
        for name in ["test.echo", "test.sleep", "test.blob"] {
            descriptors.push(ab_operations::descriptor_for(name)?);
        }
        let catalog = descriptors
            .into_iter()
            .map(|descriptor| {
                (
                    descriptor.name,
                    serde_json::json!({"scope":descriptor.scope,"schema":descriptor.input}),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        println!("{}", serde_json::to_string(&catalog)?);
        return Ok(true);
    }
    if args.verb == "status" || args.verb == "doctor" {
        let report = if args.verb == "status" {
            ab_runtime::inspection::status(home)?
        } else {
            ab_runtime::inspection::doctor(home)?
        };
        println!("{report}");
        return Ok(true);
    }
    if args.verb == "restore" {
        ab_runtime::maintenance::restore(
            home,
            &PathBuf::from(args.flag("backup", &args.value)),
            args.flag("accept-ledger-loss", "false") == "true",
        )?;
        return Ok(true);
    }
    if args.verb == "backup" {
        ab_runtime::maintenance::backup(home, &PathBuf::from(&args.value))?;
        return Ok(true);
    }
    Ok(false)
}
async fn remote(args: &Arguments, home: &std::path::Path) -> Result<(), Error> {
    if args.action == "certificate-accept" {
        ab_remote::ct::accept(home, &args.value)?;
        return Ok(());
    }
    let private = args.flag("private", "false") == "true";
    let mut via = args.flag("via", "relay");
    if private {
        if args.flags.contains_key("via") || args.flags.contains_key("domain") {
            return Err(Error::Invalid);
        }
        via = "private".into();
    }
    if args.flag("snippets", "false") == "true" {
        if args.action != "status" {
            return Err(Error::Invalid);
        }
        print!("{}", ab_remote::state::snippets(home)?);
        return Ok(());
    }
    let domain = args.flag("domain", "");
    if args.flags.contains_key("domain") && (via != "relay" || domain.is_empty()) {
        return Err(Error::Invalid);
    }
    if args.flags.contains_key("relay-url") {
        if !["on", "key"].contains(&args.action.as_str()) || via != "relay" {
            return Err(Error::Invalid);
        }
        ab_remote::state::set_relay(home, &args.flag("relay-url", ""))?;
    }
    if args.action == "key" {
        let config = ab_remote::state::read::<ab_remote::state::Config>(home, "config")?;
        println!("{}", ab_remote::relay::public_key(home, &config.relay_url)?);
        return Ok(());
    }
    if args.action == "on" && via == "relay" {
        ab_remote::state::set_domain(home, &domain)?;
    }
    if args.action != "status" && args.flags.contains_key("name") {
        ab_remote::state::annotate(home, &args.flag("name", ""))?;
    }
    let mut out = std::io::stdout();
    ab_remote::onboarding::command(
        home,
        &args.action,
        &via,
        &mut out,
        Arc::new(ab_remote::onboarding::open_link),
        std::time::Duration::from_millis(100),
    )
    .await?;
    Ok(())
}
async fn run() -> Result<(), Error> {
    let args = Arguments::parse()?;
    if args.verb == "shortcuts" {
        if args.action != "install-pack" || !args.value.is_empty() {
            return Err(Error::Invalid);
        }
        let directory = PathBuf::from(args.flag("pack", ".build/shortcuts"));
        return tokio::task::spawn_blocking(move || ab_shortcuts::cli::open_pack(&directory))
            .await
            .map_err(|_| Error::Invalid)?
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())));
    }
    let home = args
        .flags
        .get("home")
        .map_or_else(default_home, |v| Ok(PathBuf::from(v)))?;
    if product_verb(&args)? {
        return Ok(());
    }
    if args.verb == "connect" {
        ab_remote::private::connect(&args.value, &args.flag("listen", "127.0.0.1:7780")).await?;
        return Ok(());
    }
    if args.verb == "connector" {
        let result = ab_runtime::connector::command(
            &home,
            &args.action,
            &args.flag("request", ""),
            &args.flag("code", ""),
        )
        .await?;
        println!("{result}");
        return Ok(());
    }
    if args.verb == "remote" {
        return remote(&args, &home).await;
    }
    if offline(&args, &home)? {
        return Ok(());
    }
    let authority = Arc::new(ab_trust::Authority::open(&home)?);
    if manage(&args, &home, &authority)? {
        return Ok(());
    }
    match args.verb.as_str() {
        "token" => {
            let grants = args
                .flag("scope", "reminders:read,system:read")
                .split(',')
                .map(ab_trust::Grant::parse)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| Error::Invalid)?;
            let token = authority
                .create_token(grants, args.flag("remote", "false") == "true")
                .map_err(|_| Error::Operation(ab_operations::Code::Unauthorized.into()))?;
            println!("{token}");
        }
        "join" => {
            if args.value.is_empty() {
                return Err(Error::Invalid);
            }
            let listen = args.flag("listen", "");
            let result = if ab_trust::PairCode::decode(&args.value)?.remote {
                ab_replication::relay::join(&authority, &args.value)
                    .await
                    .map_err(|_| ab_trust::Error::Unauthorized)
            } else if listen.is_empty() {
                authority.join(&args.value).await
            } else {
                authority.join_listen(&args.value, &listen).await
            };
            result.map_err(|e| {
                Error::Operation(
                    if matches!(e, ab_trust::Error::Unauthorized) {
                        ab_operations::Code::Unauthorized
                    } else {
                        ab_operations::Code::Unavailable
                    }
                    .into(),
                )
            })?;
        }
        "install-credentials" => {}
        #[cfg(feature = "lab-harness")]
        "lab-credential-lifecycle" => {
            ab_runtime::inspection::credential_lifecycle(&home)?;
        }
        #[cfg(feature = "lab-harness")]
        "lab-session" => lab_session(&authority)?,
        #[cfg(feature = "lab-harness")]
        "lab-principal" => lab_principal(&args, &authority)?,
        "start" => {
            let config = args.config(home)?;
            ab_runtime::start(config, authority).await?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}

fn remote_address(home: &std::path::Path) -> Result<String, Error> {
    #[cfg(feature = "lab-harness")]
    if let Some(address) = std::env::var_os("APPLE_BRIDGE_LAB_PAIR_URL") {
        return address.into_string().map_err(|_| Error::Invalid);
    }
    let status: serde_json::Value = ab_remote::state::read(home, "status")?;
    if status["state"] != "on" {
        return Err(ab_trust::Error::Unavailable.into());
    }
    Ok(status["url"].as_str().ok_or(Error::Invalid)?.to_owned())
}
#[cfg(feature = "lab-harness")]
fn lab_session(authority: &ab_trust::Authority) -> Result<(), Error> {
    let (session, _) = authority.create_device_session("trusted")?;
    println!(
        "{}",
        serde_json::json!({"name":"ab_session_local","value":session})
    );
    Ok(())
}
#[cfg(feature = "lab-harness")]
fn lab_principal(args: &Arguments, authority: &ab_trust::Authority) -> Result<(), Error> {
    let id = args.flag("id", "");
    let fingerprint = args.flag("fingerprint", "");
    let principal = match (id.is_empty(), fingerprint.is_empty()) {
        (false, true) => authority.principal(&id)?,
        (true, false) => authority.principal_by_fingerprint(&fingerprint)?,
        _ => return Err(Error::Invalid),
    };
    println!(
        "{}",
        serde_json::json!({"id":principal.id,"kind":principal.kind,"fingerprint":principal.fingerprint,"grants":principal.grants,"revision":principal.revision,"relay2":principal.relay2()})
    );
    Ok(())
}
fn grants(args: &Arguments) -> Result<Vec<ab_trust::Grant>, Error> {
    args.flag("scope", "reminders:read,system:read")
        .split(',')
        .map(ab_trust::Grant::parse)
        .collect::<Result<_, _>>()
        .map_err(Error::from)
}
fn web_enroll(
    args: &Arguments,
    home: &std::path::Path,
    authority: &ab_trust::Authority,
) -> Result<(), Error> {
    let host = if args.flag("public", "false") == "true" {
        let status: serde_json::Value = ab_remote::state::read(home, "status")?;
        if status["state"] != "on" {
            return Err(Error::Invalid);
        }
        let url = status["url"].as_str().ok_or(Error::Invalid)?;
        if !url.starts_with("https://") {
            return Err(Error::Invalid);
        }
        url.to_owned()
    } else {
        args.flag("host", "http://localhost:7780")
    };
    let url = reqwest::Url::parse(&host).map_err(|_| Error::Invalid)?;
    let (_, rest) = host.split_once("://").ok_or(Error::Invalid)?;
    if url.host_str().is_none()
        || rest.chars().any(|c| matches!(c, '/' | '?' | '#' | '@'))
        || !(url.scheme() == "https"
            || url.scheme() == "http"
                && url.host_str() == Some("localhost")
                && rest.split(':').next() == Some("localhost"))
    {
        return Err(Error::Invalid);
    }
    let enrollment = format!("{host}/enroll#{}", authority.create_web_enroll()?);
    println!("{enrollment}");
    print!("{}", ab_runtime::product_cli::terminal_qr(&enrollment)?);
    Ok(())
}
#[derive(serde::Serialize)]
struct AgentSummary<'a> {
    id: &'a str,
    name: &'a str,
    level: &'a str,
    scopes: &'a [ab_trust::Grant],
    created_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used: Option<&'a serde_json::Value>,
}
#[derive(serde::Serialize)]
struct Created<'a> {
    agent: AgentSummary<'a>,
    token: &'a str,
}
fn agent_summary(agent: &ab_trust::Principal) -> AgentSummary<'_> {
    let text = |key: &str| {
        agent
            .extra
            .get(key)
            .and_then(serde_json::Value::as_str)
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
fn agent_command(args: &Arguments, authority: &ab_trust::Authority) -> Result<(), Error> {
    match args.action.as_str() {
        "list" => println!(
            "{}",
            serde_json::to_string(
                &authority
                    .agents()?
                    .iter()
                    .map(agent_summary)
                    .collect::<Vec<_>>()
            )?
        ),
        "create" => {
            let level = match args.flag("level", "approve").as_str() {
                "read" => ab_trust::AgentLevel::Read,
                "" | "approve" => ab_trust::AgentLevel::Approve,
                "full" => ab_trust::AgentLevel::Full,
                _ => return Err(Error::Invalid),
            };
            let (agent, token) =
                authority.create_agent(&args.flag("name", ""), level, grants(args)?)?;
            println!(
                "{}",
                serde_json::to_string(&Created {
                    agent: agent_summary(&agent),
                    token: &token,
                })?
            );
        }
        "revoke" => {
            if !args.value.starts_with("agent:") {
                return Err(Error::Invalid);
            }
            authority.revoke(&args.value)?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
fn manage(
    args: &Arguments,
    home: &std::path::Path,
    authority: &ab_trust::Authority,
) -> Result<bool, Error> {
    match args.verb.as_str() {
        "web" if args.action == "enroll" => web_enroll(args, home, authority)?,
        "web" if args.action == "lock" => authority.lock_web()?,
        "web" if args.action == "devices" => {
            println!("{}", serde_json::to_string(&authority.device_sessions()?)?);
        }
        "revoke" => authority.revoke(&args.value)?,
        "scope" => authority.change_scope(&args.value, grants(args)?)?,
        "agent" => agent_command(args, authority)?,
        "pair" => {
            let config = Config::load(home)?;
            if config.role != "gateway" {
                return Err(Error::Invalid);
            }
            let nanos = ab_runtime::duration::parse(&args.flag("expires-in", "10m"))?;
            let ttl =
                std::time::Duration::from_nanos(u64::try_from(nanos).map_err(|_| Error::Invalid)?);
            if args.flag("iphone", "false") == "true" {
                if args.flag("via-remote", "false") != "true"
                    || args.flags.contains_key("peer-addr")
                    || args.flags.contains_key("dial-replica")
                {
                    return Err(Error::Invalid);
                }
                let status: serde_json::Value = ab_remote::state::read(home, "status")?;
                if status["state"] != "on" {
                    return Err(ab_trust::Error::Unavailable.into());
                }
                let warning = ab_remote::ct::owner_warning(home, ab_remote::ct::now());
                if !warning.is_empty() {
                    eprintln!("{warning}");
                }
                let code = authority.pair_iphone_code(
                    status["url"].as_str().ok_or(Error::Invalid)?,
                    true,
                    ttl,
                )?;
                println!("{code}");
                print!("{}", ab_runtime::product_cli::terminal_qr(&code)?);
            } else {
                if args.flag("via-remote", "false") == "true" {
                    if args.flags.contains_key("peer-addr")
                        || args.flags.contains_key("dial-replica")
                    {
                        return Err(Error::Invalid);
                    }
                    let address = remote_address(home)?;
                    println!("{}", authority.pair_remote(&address, grants(args)?, ttl)?);
                    return Ok(true);
                }
                println!(
                    "{}",
                    authority.pair_with_ttl(
                        &args.flag("peer-addr", &config.peer_address),
                        &args.flag("dial-replica", ""),
                        grants(args)?,
                        ttl
                    )?
                );
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
