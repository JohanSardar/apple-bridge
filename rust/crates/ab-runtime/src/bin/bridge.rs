#![forbid(unsafe_code)]
use ab_runtime::{Config, Error, default_home};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
#[derive(Debug)]
struct Arguments {
    verb: String,
    value: String,
    flags: HashMap<String, String>,
}
impl Arguments {
    fn parse() -> Result<Self, Error> {
        let mut args = std::env::args().skip(1);
        let verb = args.next().ok_or(Error::Invalid)?;
        if verb == "token" && args.next().as_deref() != Some("create") {
            return Err(Error::Invalid);
        }
        let mut flags = HashMap::new();
        let mut value = String::new();
        while let Some(argument) = args.next() {
            if let Some(flag) = argument
                .strip_prefix("--")
                .or_else(|| argument.strip_prefix('-'))
            {
                let (key, content) = if let Some((key, content)) = flag.split_once('=') {
                    (key.to_owned(), content.to_owned())
                } else if flag == "remote" {
                    (flag.to_owned(), "true".into())
                } else {
                    (flag.to_owned(), args.next().ok_or(Error::Invalid)?)
                };
                if ![
                    "home",
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
        Ok(Self { verb, value, flags })
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
async fn run() -> Result<(), Error> {
    let args = Arguments::parse()?;
    let home = args
        .flags
        .get("home")
        .map_or_else(default_home, |v| Ok(PathBuf::from(v)))?;
    let authority = Arc::new(
        ab_trust::Authority::open(&home)
            .map_err(|_| Error::Operation(ab_operations::Code::Unavailable.into()))?,
    );
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
        "revoke" => {
            authority
                .update(|credentials| {
                    let principal = credentials
                        .principals
                        .iter_mut()
                        .find(|p| p.id == args.value)
                        .ok_or(ab_trust::Error::Invalid)?;
                    principal.extra.insert(
                        "revoked_at".into(),
                        serde_json::Value::String(jiff::Timestamp::now().to_string()),
                    );
                    principal.revision += 1;
                    Ok(())
                })
                .map_err(|_| Error::Invalid)?;
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
            result.map_err(|_| Error::Operation(ab_operations::Code::Unavailable.into()))?;
        }
        "status" => {
            let config = Config::load(&home)?;
            let state = ab_state::State::<ab_state::Replica>::open(&home)
                .map_err(|_| Error::Operation(ab_operations::Code::Unavailable.into()))?;
            println!(
                "{}",
                serde_json::json!({"role":config.role,"cursor":state.head().map_err(|_| Error::Invalid)?,"enrolled":authority.enrollment().map_err(|_| Error::Invalid)?.is_some()})
            );
        }
        "doctor" => {
            use std::os::unix::fs::PermissionsExt;
            let private = std::fs::metadata(&home)?.permissions().mode() & 0o777 == 0o700;
            println!(
                "{}",
                serde_json::json!({"private_modes":private,"credentials":{"store":"file"},"disk_encryption":"unknown","clock_skew":"unknown","versions":{"bridge":"rust"},"warnings":["replica doctor checks incomplete"]})
            );
        }
        "start" => {
            let config = Config {
                role: args.flag("role", "gateway"),
                home,
                api_address: args.flag("api-addr", "127.0.0.1:7780"),
                peer_address: args.flag("peer-addr", ""),
                listen: args.flag("listen", ""),
                socket: args.flag("socket", ""),
                retention_rows: 0,
                retention_age: 0,
            };
            config.validate()?;
            ab_runtime::start(config, authority).await?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
