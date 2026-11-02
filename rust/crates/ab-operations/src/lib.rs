#![forbid(unsafe_code)]
mod adapters;
use ab_catalog::{OperationKind, operations};
pub use adapters::{StateReads, TrustIdentity};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use std::{collections::BTreeMap, fmt::Debug, future::Future, pin::Pin, sync::Arc, time::Duration};
pub type Task<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;
pub type Visibility = BTreeMap<String, bool>;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Invalid,
    Unauthorized,
    Forbidden,
    Unavailable,
    Unsupported,
    NotFound,
    Conflict,
    Reset,
    Capacity,
    Corrupt,
    Deadline,
    Expired,
    Indeterminate,
}
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct Error {
    pub code: Code,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Box<RawValue>>,
}
impl From<Code> for Error {
    fn from(code: Code) -> Self {
        let message = match code {
            Code::Invalid => "invalid input",
            Code::Unauthorized => "credential or scope rejected",
            Code::Forbidden => "query grant rejected",
            Code::Unavailable => "peer unavailable",
            Code::Unsupported => "operation unsupported",
            Code::NotFound => "record not found",
            Code::Conflict => "state conflict",
            Code::Reset => "history must be reset",
            Code::Capacity => "capacity exceeded",
            Code::Corrupt => "invalid integrity evidence",
            Code::Deadline => "query deadline exceeded",
            Code::Expired => "credential expired",
            Code::Indeterminate => "effect cannot be established",
        };
        Self {
            code,
            message: message.into(),
            retryable: matches!(code, Code::Capacity | Code::Unavailable),
            current: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub kind: String,
    pub grants: Vec<String>,
    pub level: String,
    pub resource: String,
    pub remote: bool,
}
impl Principal {
    pub fn visibility(&self) -> Visibility {
        let mut result = Visibility::from([(format!("principal:{}", self.id), true)]);
        if self.kind == "replica" {
            result.insert("commands:all".into(), true);
        }
        for grant in &self.grants {
            if grant == "admin" {
                result.insert("admin".into(), true);
            }
            if let Some((source, "read" | "write")) = grant.split_once(':') {
                result.insert(source.into(), true);
            }
        }
        result
    }
    pub fn authorize(&self, scope: &str) -> Result<(), Error> {
        if self.grants.iter().any(|g| g == "admin" || g == scope) {
            Ok(())
        } else {
            Err(Code::Unauthorized.into())
        }
    }
}
pub trait Identity: Debug + Send + Sync {
    fn authenticate(&self, token: &str) -> Result<Principal, Error>;
    fn current(&self, principal: &Principal) -> Result<Principal, Error>;
    fn sign(&self, principal: &Principal, id: &str, hash: &str) -> Result<String, Error>;
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: String,
    pub subscription_revision: u64,
    pub sequence: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadQuery {
    pub collection: String,
    pub id: String,
    pub text: String,
    pub after: String,
    pub include_deleted: bool,
    pub include_aliases: bool,
    pub limit: usize,
}
pub trait ReadStore: Debug + Send + Sync {
    fn head(&self) -> Result<Cursor, Error>;
    fn read(&self, visibility: &Visibility, query: &ReadQuery) -> Result<Box<RawValue>, Error>;
    fn read_model(
        &self,
        visibility: &Visibility,
        name: &str,
        query: &Value,
    ) -> Result<Box<RawValue>, Error>;
    fn changes(
        &self,
        visibility: &Visibility,
        cursor: &Cursor,
    ) -> Result<Option<Box<RawValue>>, Error>;
}
pub trait Codec: Debug + Send + Sync {
    fn json(&self, raw: &[u8]) -> Result<Vec<u8>, Error>;
    fn command_hash(&self, command: &Command) -> Result<String, Error>;
    fn fingerprint(&self, raw: &[u8]) -> String;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(
        default,
        rename = "command_id",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub op: String,
    pub args: Box<RawValue>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expected_version: u64,
    #[serde(default = "zero_time")]
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admission_era: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wait_ms: u64,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}
fn zero_time() -> String {
    "0001-01-01T00:00:00Z".into()
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommandQuery {
    #[serde(rename = "command_id", skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolve: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub limit: u64,
}
#[derive(Debug)]
pub struct ForwardReply {
    pub result: Box<RawValue>,
    pub lookup_only: bool,
}
#[derive(Debug, Clone)]
pub enum GatewayRoute {
    Direct {
        address: String,
        fingerprint: String,
    },
    Feed,
}
pub trait GatewayTransport: Debug + Send + Sync {
    fn connected(&self) -> bool;
    fn exchange<'a>(
        &'a self,
        route: &'a GatewayRoute,
        delegation: String,
        body: Vec<u8>,
    ) -> Task<'a, ForwardReply>;
}
#[derive(Debug)]
pub struct RemoteGateway {
    pub route: GatewayRoute,
    pub transport: Arc<dyn GatewayTransport>,
    pub identity: Arc<dyn Identity>,
    pub store: Arc<dyn ReadStore>,
    pub codec: Arc<dyn Codec>,
}
#[derive(Serialize)]
struct Forwarded<'a> {
    method: &'a str,
    command: &'a Command,
    query: &'a CommandQuery,
}

impl RemoteGateway {
    async fn call(
        &self,
        principal: &Principal,
        method: &str,
        command: Option<&Command>,
        query: &CommandQuery,
    ) -> Result<ForwardReply, Error> {
        if !self.transport.connected() {
            return Err(Code::Unavailable.into());
        }
        let empty = adapters::empty_command()?;
        let raw = serde_json::to_vec(&Forwarded {
            method,
            command: command.unwrap_or(&empty),
            query,
        })
        .map_err(|_| Error::from(Code::Invalid))?;
        let body = self.codec.json(&raw)?;
        let hash = if let Some(command) = command {
            self.codec.command_hash(command)?
        } else {
            self.codec.fingerprint(&body)
        };
        let id = command.map_or(query.id.as_str(), |c| c.id.as_str());
        let delegation = self.identity.sign(principal, id, &hash)?;
        self.transport.exchange(&self.route, delegation, body).await
    }
    async fn wait(&self, response: ForwardReply) -> Result<Box<RawValue>, Error> {
        let value: Value =
            serde_json::from_str(response.result.get()).map_err(|_| Error::from(Code::Corrupt))?;
        if !response.lookup_only && value["commit_seq"].as_u64().unwrap_or(0) > 0 {
            let cursor: Cursor = serde_json::from_value(value["cursor"].clone())
                .map_err(|_| Error::from(Code::Corrupt))?;
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let local = self.store.head()?;
                    if local.epoch != cursor.epoch
                        || local.subscription_revision != cursor.subscription_revision
                    {
                        return Err(Error::from(Code::Reset));
                    }
                    if local.sequence >= cursor.sequence {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .map_err(|_| Error::from(Code::Deadline))??;
        }
        Ok(response.result)
    }
    pub async fn submit(
        &self,
        principal: &Principal,
        mut command: Command,
    ) -> Result<Box<RawValue>, Error> {
        if !valid_command_id(&command.id) || command.wait_ms > 15000 {
            return Err(Code::Invalid.into());
        }
        if principal.kind == "agent" && principal.level == "read" {
            return Err(Code::Unauthorized.into());
        }
        command.origin = if principal.kind == "device" {
            "interactive".into()
        } else {
            String::new()
        };
        let query = CommandQuery {
            id: command.id.clone(),
            ..CommandQuery::default()
        };
        let response = match self
            .call(
                principal,
                "submit",
                Some(&command),
                &CommandQuery::default(),
            )
            .await
        {
            Err(Error {
                code: Code::Unavailable,
                ..
            }) => self.call(principal, "status", None, &query).await?,
            result => result?,
        };
        self.wait(response).await
    }
    pub async fn status(
        &self,
        principal: &Principal,
        query: &CommandQuery,
    ) -> Result<Box<RawValue>, Error> {
        self.wait(self.call(principal, "status", None, query).await?)
            .await
    }
}
pub fn valid_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
#[derive(Debug)]
pub enum CommandTarget {
    Remote(Arc<RemoteGateway>),
}
#[derive(Debug)]
pub struct Service {
    pub store: Arc<dyn ReadStore>,
    pub identity: Arc<dyn Identity>,
    pub target: CommandTarget,
    pub codec: Arc<dyn Codec>,
}
impl Service {
    pub async fn invoke(
        &self,
        principal: &Principal,
        name: &str,
        input: &[u8],
    ) -> Result<Box<RawValue>, Error> {
        let principal = self.identity.current(principal)?;
        let descriptors = operations().map_err(|_| Error::from(Code::Corrupt))?;
        let descriptor = descriptors
            .iter()
            .find(|d| d.name == name)
            .ok_or(Code::Unsupported)?;
        let CommandTarget::Remote(target) = &self.target;
        if descriptor.kind == OperationKind::Command {
            principal.authorize(&descriptor.scope)?;
            let mut command: Command =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            if !command.op.is_empty() && command.op != name {
                return Err(Code::Invalid.into());
            }
            command.op = name.into();
            if command.id.is_empty() {
                command.id = format!("cmd_{}", ulid::Ulid::new());
            }
            return target.submit(&principal, command).await;
        }
        if name == "commands.status" || name == "commands.list" {
            let query: CommandQuery =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            if query.limit > 1000 || (name == "commands.status" && !valid_command_id(&query.id)) {
                return Err(Code::Invalid.into());
            }
            return if name == "commands.status" {
                target.status(&principal, &query).await
            } else {
                Ok(target.call(&principal, "list", None, &query).await?.result)
            };
        }
        if descriptor.kind == OperationKind::Query {
            return Err(Code::Unavailable.into());
        }
        let visibility = principal.visibility();
        if descriptor.read_model {
            let query: Value =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            return self.store.read_model(&visibility, name, &query);
        }
        let mut query: ReadQuery =
            serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
        if query.limit > 1000
            || (name == "records.get" && (query.collection.is_empty() || query.id.is_empty()))
            || (name == "search" && query.text.is_empty())
        {
            return Err(Code::Invalid.into());
        }
        if name == "sources.status" {
            query = ReadQuery {
                collection: "system.source".into(),
                ..ReadQuery::default()
            };
        }
        self.store.read(&visibility, &query)
    }
}
