use super::{
    Failure, Outcome,
    tree::{
        AgentAction, AgentCreateArgs, ConnectArgs, ConnectorAction, JoinArgs, PairArgs,
        RemoteAction, RemoteArgs, RestoreArgs, ScopeArgs, SelftestArgs, ShortcutsAction, StartArgs,
        TokenAction, WebAction,
    },
};
use crate::{Config, Error, product_cli};
use ab_trust::{Authority, Grant};
use std::{path::Path, sync::Arc};

const REMOTE_NOT_ON: &str = "Remote access is off.";
const REMOTE_NOT_ON_NEXT: &str = "Run `bridge remote on` first.";

fn authority(home: &Path) -> Outcome<Arc<Authority>> {
    Ok(Arc::new(Authority::open(home).map_err(Error::from)?))
}

fn grants(scope: &str) -> Outcome<Vec<Grant>> {
    scope
        .split(',')
        .map(|name| {
            Grant::parse(name).map_err(|_| {
                Failure::new(format!("`{name}` is not a known scope."))
                    .next("Scopes look like messages:read or calendar:write.")
            })
        })
        .collect()
}

pub fn permission_request(name: &str, helper_app: Option<&str>) -> Outcome {
    if !product_cli::PERMISSIONS.contains(&name) {
        return Err(Failure::new(format!("`{name}` is not a permission that can be requested."))
            .next(format!(
                "Choose one of: {}. For Messages, Contacts, Calendar and Reminders run `bridge permissions guide`.",
                product_cli::PERMISSIONS.join(", ")
            )));
    }
    product_cli::permissions_request(
        Path::new("open"),
        &product_cli::default_report_dir(),
        helper_app.unwrap_or_default(),
        name,
        None,
    )?;
    Ok(())
}

pub fn selftest(args: &SelftestArgs) -> Outcome {
    let mut app = args.helper_app.clone().unwrap_or_default();
    if app.is_empty()
        && let Some(bundle) = product_cli::detect_helper_app()
    {
        app = bundle.to_string_lossy().into_owned();
    }
    let result = product_cli::messages_selftest(
        Path::new("open"),
        &product_cli::default_report_dir(),
        &app,
        args.force,
        !args.no_ui,
        None,
    )?;
    println!("{}", result.data);
    if !result.proven() {
        return Err(Failure::new(format!(
            "messages self-test {}; failed operations remain unproven",
            result.state
        )));
    }
    Ok(())
}

pub async fn connector(home: &Path, action: ConnectorAction) -> Outcome {
    let (name, request, code) = match &action {
        ConnectorAction::Pending => ("pending", "", ""),
        ConnectorAction::Allow(decision) => {
            ("allow", decision.request.as_str(), decision.code.as_str())
        }
        ConnectorAction::Deny(decision) => {
            ("deny", decision.request.as_str(), decision.code.as_str())
        }
    };
    let result = crate::connector::command(home, name, request, code)
        .await
        .map_err(|_| super::connector_failure(home, &action))?;
    println!("{result}");
    Ok(())
}

fn inspection_failure(home: &Path, error: Error) -> Failure {
    if home.join("state.db").exists() {
        Failure::from(error)
    } else {
        Failure::not_set_up(home)
    }
}

pub fn status(home: &Path) -> Outcome {
    let report =
        crate::inspection::status(home).map_err(|error| inspection_failure(home, error))?;
    println!("{report}");
    Ok(())
}

pub fn doctor(home: &Path) -> Outcome {
    let report =
        crate::inspection::doctor(home).map_err(|error| inspection_failure(home, error))?;
    println!("{report}");
    Ok(())
}

pub fn backup(home: &Path, path: &Path) -> Outcome {
    crate::maintenance::backup(home, path).map_err(|_| {
        Failure::new(format!("Could not back up to {}.", path.display())).next(
            "The gateway must be running (`bridge up`) and the folder must exist and be writable.",
        )
    })
}

pub fn restore(home: &Path, args: RestoreArgs) -> Outcome {
    let backup = args.backup.or(args.path).unwrap_or_default();
    crate::maintenance::restore(home, &backup, args.accept_ledger_loss).map_err(|error| {
        if matches!(error, Error::Invalid) {
            Failure::new(format!("{} could not be restored.", backup.display()))
                .next("Stop the gateway with `bridge down`, and check the file is a backup.")
        } else {
            Failure::from(error)
        }
    })
}

fn remote_flags(args: &RemoteArgs) -> Outcome<String> {
    let mut via = args.via.clone().unwrap_or_else(|| "relay".into());
    if args.private {
        if args.via.is_some() || args.domain.is_some() {
            return Err(Failure::new(
                "--private cannot be used with --via or --domain.",
            ));
        }
        via = "private".into();
    }
    if args.snippets && args.action != RemoteAction::Status {
        return Err(Failure::new(
            "--snippets only works with `bridge remote status`.",
        ));
    }
    if let Some(domain) = &args.domain
        && (via != "relay" || domain.is_empty())
    {
        return Err(Failure::new(
            "--domain needs a domain name and the relay route.",
        ));
    }
    if args.relay_url.is_some()
        && (!matches!(args.action, RemoteAction::On | RemoteAction::Key) || via != "relay")
    {
        return Err(Failure::new(
            "--relay-url only works with `remote on` or `remote key` on the relay route.",
        ));
    }
    Ok(via)
}

pub async fn remote(home: &Path, args: &RemoteArgs) -> Outcome {
    if args.action == RemoteAction::CertificateAccept {
        ab_remote::ct::accept(home, args.value.as_deref().unwrap_or_default())?;
        return Ok(());
    }
    let via = remote_flags(args)?;
    if args.snippets {
        print!("{}", ab_remote::state::snippets(home)?);
        return Ok(());
    }
    if let Some(url) = &args.relay_url {
        ab_remote::state::set_relay(home, url)?;
    }
    if args.action == RemoteAction::Key {
        let config = ab_remote::state::read::<ab_remote::state::Config>(home, "config")?;
        println!("{}", ab_remote::relay::public_key(home, &config.relay_url)?);
        return Ok(());
    }
    if args.action == RemoteAction::On && via == "relay" {
        ab_remote::state::set_domain(home, args.domain.as_deref().unwrap_or_default())?;
    }
    if args.action != RemoteAction::Status
        && let Some(name) = &args.name
    {
        ab_remote::state::annotate(home, name)?;
    }
    let mut out = std::io::stdout();
    ab_remote::onboarding::command(
        home,
        args.action.as_str(),
        &via,
        &mut out,
        Arc::new(ab_remote::onboarding::open_link),
        std::time::Duration::from_millis(100),
    )
    .await?;
    Ok(())
}

pub async fn connect(args: &ConnectArgs) -> Outcome {
    ab_remote::private::connect(&args.code, &args.listen).await?;
    Ok(())
}

pub async fn shortcuts(action: ShortcutsAction) -> Outcome {
    let ShortcutsAction::InstallPack { pack } = action;
    tokio::task::spawn_blocking(move || ab_shortcuts::cli::open_pack(&pack))
        .await
        .map_err(|_| Failure::new("The Shortcuts pack could not be opened."))?
        .map_err(|error| Failure::new(error.to_string()))
}

pub fn token(home: &Path, action: TokenAction) -> Outcome {
    let TokenAction::Create { scope, remote } = action;
    let token = authority(home)?
        .create_token(grants(&scope)?, remote)
        .map_err(|_| {
            Failure::new("This home cannot grant that access.")
                .next("A replica can only hand out access it was given itself.")
        })?;
    println!("{token}");
    Ok(())
}

pub async fn join(home: &Path, args: &JoinArgs) -> Outcome {
    let authority = authority(home)?;
    let refused = |remote_failure: bool| {
        if remote_failure {
            Failure::new("unauthorized: the gateway refused this pairing code.")
                .next("Pairing codes expire. Create a new one with `bridge pair` on the gateway.")
        } else {
            Failure::new("unavailable: the gateway could not be reached.").next(
                "Check it is running and the address in the code is reachable, then try again.",
            )
        }
    };
    let code = ab_trust::PairCode::decode(&args.code).map_err(|_| {
        Failure::new("That is not a valid pairing code.")
            .next("Copy the whole code that `bridge pair` printed.")
    })?;
    let result = if code.remote {
        ab_replication::relay::join(&authority, &args.code)
            .await
            .map_err(|_| ab_trust::Error::Unauthorized)
    } else if let Some(listen) = &args.listen {
        authority.join_listen(&args.code, listen).await
    } else {
        authority.join(&args.code).await
    };
    result
        .map(drop)
        .map_err(|error| refused(matches!(error, ab_trust::Error::Unauthorized)))
}

pub fn install_credentials(home: &Path) -> Outcome {
    authority(home)?;
    Ok(())
}

pub async fn start(home: &Path, args: StartArgs) -> Outcome {
    let authority = authority(home)?;
    let retention_age = crate::duration::parse(&args.retention_age).map_err(|_| {
        Failure::new(format!("`{}` is not a valid duration.", args.retention_age))
            .next("Durations look like 90s, 10m or 168h.")
    })?;
    let config = Config {
        role: args.role,
        home: home.to_owned(),
        api_address: args.api_addr,
        peer_address: args.peer_addr,
        listen: args.listen,
        socket: args.socket,
        helper_app: crate::bundle::helper_app(
            &args.helper_app,
            std::env::current_exe().ok().as_deref(),
        ),
        helper_lab_commands: args.helper_lab_commands,
        retention_rows: args.retention_rows,
        retention_age,
        blob_cap: args.blob_cap,
        blob_grace_ms: args.blob_grace_ms,
    };
    let address_is_valid = config.validate().is_ok();
    if !address_is_valid {
        return Err(Failure::new(
            "--role must be gateway or replica, and --api-addr must be a loopback HOST:PORT.",
        ));
    }
    crate::start(config, authority).await.map_err(Failure::from)
}

fn remote_address(home: &Path) -> Outcome<String> {
    #[cfg(feature = "lab-harness")]
    if let Some(address) = std::env::var_os("APPLE_BRIDGE_LAB_PAIR_URL") {
        return address
            .into_string()
            .map_err(|_| Failure::new("APPLE_BRIDGE_LAB_PAIR_URL is not text."));
    }
    remote_url(home)
}

fn remote_url(home: &Path) -> Outcome<String> {
    let off = || Failure::new(REMOTE_NOT_ON).next(REMOTE_NOT_ON_NEXT);
    let status: serde_json::Value = ab_remote::state::read(home, "status").map_err(|_| off())?;
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

fn web_enroll(home: &Path, authority: &Authority, host: String, public: bool) -> Outcome {
    let host = if public {
        let url = remote_url(home)?;
        if !url.starts_with("https://") {
            return Err(Failure::new(REMOTE_NOT_ON).next(REMOTE_NOT_ON_NEXT));
        }
        url
    } else {
        host
    };
    if !valid_web_host(&host) {
        return Err(Failure::new(format!("`{host}` cannot host sign-in pages."))
            .next("Use https://HOST, or http://localhost:PORT on this Mac."));
    }
    let enrollment = format!("{host}/enroll#{}", authority.create_web_enroll()?);
    println!("{enrollment}");
    print!("{}", product_cli::terminal_qr(&enrollment)?);
    Ok(())
}

pub fn web(home: &Path, action: WebAction) -> Outcome {
    let authority = authority(home)?;
    match action {
        WebAction::Enroll { host, public } => web_enroll(home, &authority, host, public),
        WebAction::Lock => Ok(authority.lock_web()?),
        WebAction::Devices => {
            println!("{}", serde_json::to_string(&authority.device_sessions()?)?);
            Ok(())
        }
    }
}

pub fn revoke(home: &Path, id: &str) -> Outcome {
    authority(home)?.revoke(id).map_err(|_| unknown_id(id))
}

pub fn scope(home: &Path, args: &ScopeArgs) -> Outcome {
    authority(home)?
        .change_scope(&args.id, grants(&args.scope)?)
        .map_err(|_| unknown_id(&args.id))
}

fn unknown_id(id: &str) -> Failure {
    Failure::new(format!("No credential with id `{id}`.")).next("Run `bridge agent list`.")
}

#[derive(serde::Serialize)]
struct AgentSummary<'a> {
    id: &'a str,
    name: &'a str,
    level: &'a str,
    scopes: &'a [Grant],
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

fn agent_create(authority: &Authority, args: &AgentCreateArgs) -> Outcome {
    let level = match args.level.as_str() {
        "read" => ab_trust::AgentLevel::Read,
        "" | "approve" => ab_trust::AgentLevel::Approve,
        "full" => ab_trust::AgentLevel::Full,
        other => {
            return Err(Failure::new(format!("`{other}` is not an agent level."))
                .next("Choose read, approve or full."));
        }
    };
    let (agent, token) = authority
        .create_agent(&args.name, level, grants(&args.scope)?)
        .map_err(|_| {
            Failure::new("That agent could not be created.")
                .next("Give it a name of 1 to 120 characters and scopes such as messages:read.")
        })?;
    println!(
        "{}",
        serde_json::to_string(&Created {
            agent: agent_summary(&agent),
            token: &token,
        })?
    );
    Ok(())
}

pub fn agent(home: &Path, action: AgentAction) -> Outcome {
    let authority = authority(home)?;
    match action {
        AgentAction::List => {
            println!(
                "{}",
                serde_json::to_string(
                    &authority
                        .agents()?
                        .iter()
                        .map(agent_summary)
                        .collect::<Vec<_>>()
                )?
            );
            Ok(())
        }
        AgentAction::Create(args) => agent_create(&authority, &args),
        AgentAction::Revoke(args) => {
            if !args.id.starts_with("agent:") {
                return Err(Failure::new(format!("`{}` is not an agent id.", args.id))
                    .next("Agent ids start with agent:. Run `bridge agent list`."));
            }
            authority.revoke(&args.id).map_err(|_| unknown_id(&args.id))
        }
    }
}

fn pair_iphone(
    home: &Path,
    authority: &Authority,
    args: &PairArgs,
    ttl: std::time::Duration,
) -> Outcome {
    if !args.via_remote || args.peer_addr.is_some() || args.dial_replica.is_some() {
        return Err(Failure::new(
            "--iphone needs --via-remote and cannot use --peer-addr or --dial-replica.",
        ));
    }
    let url = remote_url(home)?;
    let warning = ab_remote::ct::owner_warning(home, ab_remote::ct::now());
    if !warning.is_empty() {
        eprintln!("{warning}");
    }
    let code = authority.pair_iphone_code(&url, true, ttl)?;
    println!("{code}");
    print!("{}", product_cli::terminal_qr(&code)?);
    Ok(())
}

pub fn pair(home: &Path, args: &PairArgs) -> Outcome {
    let authority = authority(home)?;
    let config = Config::load(home).map_err(|_| Failure::not_set_up(home))?;
    if config.role != "gateway" {
        return Err(Failure::new("Only a gateway can create pairing codes."));
    }
    let nanos = crate::duration::parse(&args.expires_in);
    let ttl = nanos
        .ok()
        .and_then(|nanos| u64::try_from(nanos).ok())
        .map(std::time::Duration::from_nanos)
        .ok_or_else(|| {
            Failure::new(format!("`{}` is not a valid duration.", args.expires_in))
                .next("Durations look like 90s, 10m or 168h.")
        })?;
    if args.iphone {
        return pair_iphone(home, &authority, args, ttl);
    }
    if args.via_remote {
        if args.peer_addr.is_some() || args.dial_replica.is_some() {
            return Err(Failure::new(
                "--via-remote cannot use --peer-addr or --dial-replica.",
            ));
        }
        let address = remote_address(home)?;
        println!(
            "{}",
            authority.pair_remote(&address, grants(&args.scope)?, ttl)?
        );
        return Ok(());
    }
    println!(
        "{}",
        authority.pair_with_ttl(
            args.peer_addr.as_deref().unwrap_or(&config.peer_address),
            args.dial_replica.as_deref().unwrap_or_default(),
            grants(&args.scope)?,
            ttl
        )?
    );
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_operation_catalog() -> Outcome {
    let mut descriptors = ab_catalog::operations()?;
    for name in ["test.echo", "test.sleep", "test.blob"] {
        descriptors.push(ab_operations::descriptor_for(name).map_err(Error::from)?);
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
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_credential_lifecycle(home: &Path) -> Outcome {
    authority(home)?;
    crate::inspection::credential_lifecycle(home)?;
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_session(home: &Path) -> Outcome {
    let (session, _) = authority(home)?.create_device_session("trusted")?;
    println!(
        "{}",
        serde_json::json!({"name":"ab_session_local","value":session})
    );
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_principal(home: &Path, args: &super::tree::LabPrincipalArgs) -> Outcome {
    let authority = authority(home)?;
    let principal = match (&args.id, &args.fingerprint) {
        (Some(id), None) => authority.principal(id)?,
        (None, Some(fingerprint)) => authority.principal_by_fingerprint(fingerprint)?,
        _ => return Err(Failure::new("Pass exactly one of --id or --fingerprint.")),
    };
    println!(
        "{}",
        serde_json::json!({"id":principal.id,"kind":principal.kind,"fingerprint":principal.fingerprint,"grants":principal.grants,"revision":principal.revision,"relay2":principal.relay2()})
    );
    Ok(())
}
