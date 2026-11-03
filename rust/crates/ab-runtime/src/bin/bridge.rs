#![forbid(unsafe_code)]
use ab_runtime::{Config, Error, default_home};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
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
        let action = if ["agent", "remote", "web"].contains(&verb.as_str()) {
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
                } else if [
                    "remote",
                    "public",
                    "iphone",
                    "via-remote",
                    "helper-lab-commands",
                    "accept-ledger-loss",
                ]
                .contains(&flag)
                {
                    (flag.to_owned(), "true".into())
                } else {
                    (flag.to_owned(), args.next().ok_or(Error::Invalid)?)
                };
                if ![
                    "home",
                    "host",
                    "public",
                    "helper-app",
                    "helper-lab-commands",
                    "via",
                    "name",
                    "level",
                    "expires-in",
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
                ]
                .contains(&key.as_str())
                {
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
            helper_app: self.flag("helper-app", ""),
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
    if args.verb == "remote" {
        println!(
            "{}",
            ab_remote::state::command(home, &args.action, &args.flag("via", "relay"))?
        );
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
async fn run() -> Result<(), Error> {
    let args = Arguments::parse()?;
    let home = args
        .flags
        .get("home")
        .map_or_else(default_home, |v| Ok(PathBuf::from(v)))?;
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
            let result = if listen.is_empty() {
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
        #[cfg(feature = "lab-harness")]
        "lab-credential-lifecycle" => {
            ab_runtime::inspection::credential_lifecycle(&home)?;
        }
        "start" => {
            let config = args.config(home)?;
            ab_runtime::start(config, authority).await?;
        }
        _ => return Err(Error::Invalid),
    }
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
    println!("{host}/enroll#{}", authority.create_web_enroll()?);
    Ok(())
}
fn agent_summary(agent: &ab_trust::Principal) -> serde_json::Value {
    let mut summary = serde_json::json!({"id":agent.id,"name":agent.extra.get("name"),"level":agent.level,"scopes":agent.grants,"created_at":agent.extra.get("created_at")});
    for key in ["revoked_at", "last_used"] {
        if let Some(value) = agent.extra.get(key).filter(|value| !value.is_null()) {
            summary[key] = value.clone();
        }
    }
    summary
}
fn manage(
    args: &Arguments,
    home: &std::path::Path,
    authority: &ab_trust::Authority,
) -> Result<bool, Error> {
    match args.verb.as_str() {
        "web" if args.action == "enroll" => web_enroll(args, home, authority)?,
        "revoke" => authority.revoke(&args.value)?,
        "scope" => authority.change_scope(&args.value, grants(args)?)?,
        "agent" => match args.action.as_str() {
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
                    serde_json::json!({"agent":agent_summary(&agent),"token":token})
                );
            }
            "revoke" => {
                if !args.value.starts_with("agent:") {
                    return Err(Error::Invalid);
                }
                authority.revoke(&args.value)?;
            }
            _ => return Err(Error::Invalid),
        },
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
                println!(
                    "{}",
                    authority.pair_iphone_code(
                        status["url"].as_str().ok_or(Error::Invalid)?,
                        true,
                        ttl
                    )?
                );
            } else {
                if args.flag("via-remote", "false") == "true" {
                    return Err(ab_trust::Error::Unavailable.into());
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
