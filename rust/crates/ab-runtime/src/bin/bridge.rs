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
    fn config(&self, home: PathBuf) -> Result<Config, Error> {
        Ok(Config {
            role: self.flag("role", "gateway"),
            home,
            api_address: self.flag("api-addr", "127.0.0.1:7780"),
            peer_address: self.flag("peer-addr", ""),
            listen: self.flag("listen", ""),
            socket: self.flag("socket", ""),
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
async fn run() -> Result<(), Error> {
    let args = Arguments::parse()?;
    let home = args
        .flags
        .get("home")
        .map_or_else(default_home, |v| Ok(PathBuf::from(v)))?;
    if args.verb == "status" || args.verb == "doctor" {
        let report = if args.verb == "status" {
            ab_runtime::inspection::status(&home)?
        } else {
            ab_runtime::inspection::doctor(&home)?
        };
        println!("{report}");
        return Ok(());
    }
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
            config.validate()?;
            ab_runtime::start(config, authority).await?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
