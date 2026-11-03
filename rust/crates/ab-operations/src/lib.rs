#![forbid(unsafe_code)]
mod adapters;
mod gateway_store;
mod local_query;
pub use gateway_store::GatewayStore;
pub use local_query::LocalQueryStore;
pub mod local;
pub mod peer;
pub use ab_trust::domain;
mod legacy;
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
    IdReused,
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
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        f.write_str(name.as_str().ok_or(std::fmt::Error)?)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: Code,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Box<RawValue>>,
    pub retryable: bool,
}
#[derive(Debug, Serialize)]
pub struct Problem<'a> {
    pub error: &'a Error,
}
impl From<Code> for Error {
    fn from(code: Code) -> Self {
        let message = match code {
            Code::IdReused => "command ID already identifies another request",
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
    pub fn is_agent(&self) -> bool {
        self.kind == "agent"
            || self.kind == "delegated"
                && self
                    .id
                    .split_once('/')
                    .is_some_and(|(_, local)| local.starts_with("agent:"))
    }
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
    fn approval_principal(
        &self,
        _id: &str,
        _command_id: &str,
        _hash: &str,
    ) -> Result<Principal, Error> {
        Err(Code::Unavailable.into())
    }
    fn iphones(&self) -> Result<Box<RawValue>, Error> {
        Err(Code::Unavailable.into())
    }
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
    fn cached_query_blob<'a>(
        &'a self,
        _principal: &'a Principal,
        _id: &'a str,
        _digest: &'a str,
    ) -> Task<'a, Vec<u8>> {
        Box::pin(async { Err(Code::NotFound.into()) })
    }
    fn cache_blob<'a>(&'a self, _digest: &'a str, _bytes: Vec<u8>) -> Task<'a, Vec<u8>> {
        Box::pin(async { Err(Code::Unavailable.into()) })
    }

    fn save_query_blobs(
        &self,
        _principal: &str,
        _id: &str,
        _scope: &str,
        _blobs: &Value,
    ) -> Result<(), Error> {
        Err(Code::Unavailable.into())
    }
    fn query_blob_size(
        &self,
        _principal: &Principal,
        _id: &str,
        _digest: &str,
    ) -> Result<u64, Error> {
        Err(Code::Forbidden.into())
    }

    fn blob<'a>(
        &'a self,
        _visibility: &'a Visibility,
        _record: &'a str,
        _digest: &'a str,
    ) -> Task<'a, Vec<u8>> {
        Box::pin(async { Err(Code::Unavailable.into()) })
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeQuery {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub op: String,
    pub args: Box<RawValue>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub deadline_ms: u64,
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
    #[serde(skip_serializing_if = "str::is_empty")]
    blob_digest: &'a str,
    #[serde(skip_serializing_if = "is_zero")]
    blob_offset: u64,
    method: &'a str,
    command: &'a Command,
    query: &'a CommandQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_query: Option<&'a NativeQuery>,
}

impl RemoteGateway {
    pub async fn query_blob(
        &self,
        principal: &Principal,
        id: &str,
        digest: &str,
    ) -> Result<Vec<u8>, Error> {
        use base64::Engine;
        let size = self.store.query_blob_size(principal, id, digest)?;
        if let Ok(bytes) = self.store.cached_query_blob(principal, id, digest).await {
            return Ok(bytes);
        }
        if size > 64 << 20 {
            return Err(Code::Invalid.into());
        }
        let mut bytes = Vec::new();
        loop {
            let empty = adapters::empty_command()?;
            let query = NativeQuery {
                id: id.into(),
                op: String::new(),
                args: RawValue::from_string("null".into()).map_err(|_| Code::Corrupt)?,
                deadline_ms: 0,
            };
            let raw = serde_json::to_vec(&Forwarded {
                blob_digest: digest,
                blob_offset: u64::try_from(bytes.len()).map_err(|_| Code::Corrupt)?,
                method: "query_blob",
                command: &empty,
                query: &CommandQuery::default(),
                native_query: Some(&query),
            })
            .map_err(|_| Code::Corrupt)?;
            let body = self.codec.json(&raw)?;
            let proof = self
                .identity
                .sign(principal, id, &self.codec.fingerprint(&body))?;
            let reply = self.transport.exchange(&self.route, proof, body).await?;
            let encoded: String =
                serde_json::from_str(reply.result.get()).map_err(|_| Code::Corrupt)?;
            let chunk = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| Code::Corrupt)?;
            if chunk.len() > 65536
                || bytes.len() + chunk.len() > usize::try_from(size).map_err(|_| Code::Corrupt)?
            {
                return Err(Code::Corrupt.into());
            }
            let done = chunk.len() < 65536;
            bytes.extend(chunk);
            if done {
                break;
            }
        }
        if u64::try_from(bytes.len()).map_err(|_| Code::Corrupt)? != size
            || self.codec.fingerprint(&bytes) != digest
        {
            return Err(Code::Corrupt.into());
        }
        self.store.cache_blob(digest, bytes).await
    }

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
            blob_digest: "",
            blob_offset: 0,
            method,
            command: command.unwrap_or(&empty),
            query,
            native_query: None,
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
                    if local.epoch != cursor.epoch {
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
            Err(
                error @ Error {
                    code: Code::Unavailable,
                    ..
                },
            ) => match self.call(principal, "status", None, &query).await {
                Ok(response) => response,
                Err(_) => return Err(error),
            },
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
    Local(Arc<local::LocalGateway>),
}
#[derive(Debug)]
pub struct Service {
    pub store: Arc<dyn ReadStore>,
    pub identity: Arc<dyn Identity>,
    pub target: CommandTarget,
    pub codec: Arc<dyn Codec>,
}
impl Service {
    async fn native_query(
        &self,
        principal: &Principal,
        name: &str,
        input: &[u8],
        scope: &str,
        target: &RemoteGateway,
    ) -> Result<Box<RawValue>, Error> {
        principal
            .authorize(scope)
            .map_err(|_| Error::from(Code::Forbidden))?;
        let mut query: NativeQuery = serde_json::from_slice(input).map_err(|_| Code::Invalid)?;
        if !query.op.is_empty() && query.op != name {
            return Err(Code::Invalid.into());
        }
        query.op = name.into();
        if query.id.is_empty() {
            query.id = format!("qry_{}", ulid::Ulid::new());
        }
        if !valid_command_id(&query.id)
            || !query.id.starts_with("qry_")
            || query.id.len() <= 4
            || query.deadline_ms > 60000
        {
            return Err(Code::Invalid.into());
        }
        if query.deadline_ms == 0 {
            query.deadline_ms = 1000;
        }
        if !target.transport.connected() {
            return Err(Code::Unavailable.into());
        }
        let empty = adapters::empty_command()?;
        let bytes = serde_json::to_vec(&Forwarded {
            blob_digest: "",
            blob_offset: 0,
            method: "query",
            command: &empty,
            query: &CommandQuery::default(),
            native_query: Some(&query),
        })
        .map_err(|_| Code::Corrupt)?;
        let body = self.codec.json(&bytes)?;
        let proof = self
            .identity
            .sign(principal, &query.id, &self.codec.fingerprint(&body))?;
        let result = tokio::time::timeout(
            Duration::from_millis(query.deadline_ms),
            target.transport.exchange(&target.route, proof, body),
        )
        .await
        .map_err(|_| Code::Deadline)?
        .map(|reply| reply.result)?;
        let output: Value = serde_json::from_str(result.get()).map_err(|_| Code::Corrupt)?;
        if output["blobs"].as_array().is_some_and(|b| !b.is_empty()) {
            self.store
                .save_query_blobs(&principal.id, &query.id, scope, &output["blobs"])?;
        }
        Ok(result)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn invoke(
        &self,
        principal: &Principal,
        name: &str,
        input: &[u8],
    ) -> Result<Box<RawValue>, Error> {
        let principal = self.identity.current(principal)?;
        let descriptor = descriptor_for(name)?;
        if descriptor.kind == OperationKind::Command {
            principal.authorize(&descriptor.scope)?;
            let mut command: Command =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            if !command.op.is_empty() && command.op != name {
                return Err(Code::Invalid.into());
            }
            command.op = name.into();
            let args: Value =
                serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
            if command.op.starts_with("calendar.") && args["span"] == "all" {
                return Err(Code::Unsupported.into());
            }
            validate_shape(&descriptor.input["properties"]["args"], &args)?;
            if !args.is_object() {
                return Err(Code::Invalid.into());
            }
            if descriptor.input["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == "expected_version"))
                && command.expected_version == 0
            {
                return Err(Code::Invalid.into());
            }
            if args
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|title| title.trim().is_empty())
            {
                return Err(Code::Invalid.into());
            }
            if command.id.is_empty() {
                command.id = format!("cmd_{}", ulid::Ulid::new());
            }
            return match &self.target {
                CommandTarget::Remote(target) => target.submit(&principal, command).await,
                CommandTarget::Local(target) => target.submit(&principal, command).await,
            };
        }
        if name == "commands.status" || name == "commands.list" {
            let query: CommandQuery =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            if query.limit > 1000 || (name == "commands.status" && !valid_command_id(&query.id)) {
                return Err(Code::Invalid.into());
            }
            return match &self.target {
                CommandTarget::Remote(target) if name == "commands.status" => {
                    target.status(&principal, &query).await
                }
                CommandTarget::Remote(target) => {
                    Ok(target.call(&principal, "list", None, &query).await?.result)
                }
                CommandTarget::Local(target) if name == "commands.status" => {
                    target.status(&principal, &query).await
                }
                CommandTarget::Local(target) => target.list(&principal, query.limit),
            };
        }
        if descriptor.kind == OperationKind::Query {
            return match &self.target {
                CommandTarget::Remote(target) => {
                    self.native_query(&principal, name, input, &descriptor.scope, target)
                        .await
                }
                CommandTarget::Local(target) => {
                    let mut query: NativeQuery =
                        serde_json::from_slice(input).map_err(|_| Code::Invalid)?;
                    if !query.op.is_empty() && query.op != name {
                        return Err(Code::Invalid.into());
                    }
                    query.op = name.into();
                    target.query(&principal, query).await
                }
            };
        }
        if name == "iphone.devices" {
            principal.authorize("iphone:read")?;
            let input: Value = serde_json::from_slice(input).map_err(|_| Code::Invalid)?;
            if !input.as_object().is_some_and(serde_json::Map::is_empty) {
                return Err(Code::Invalid.into());
            }
            return self.identity.iphones();
        }
        let visibility = principal.visibility();
        if descriptor.read_model
            || matches!(
                name,
                "contacts.lookup" | "sources.status" | "replica.status" | "health.summary"
            )
        {
            let query: Value =
                serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
            return self.store.read_model(&visibility, name, &query);
        }
        let query: ReadQuery =
            serde_json::from_slice(input).map_err(|_| Error::from(Code::Invalid))?;
        if query.limit > 10000 {
            return Err(Code::Invalid.into());
        }
        if query.collection == "system.replica" {
            return self.store.read_model(
                &visibility,
                "replica.status",
                &Value::Object(serde_json::Map::new()),
            );
        }
        if (name == "records.get" && (query.collection.is_empty() || query.id.is_empty()))
            || (name == "search" && query.text.is_empty())
        {
            return Err(Code::Invalid.into());
        }
        self.store.read(&visibility, &query)
    }
}

fn validate_shape(schema: &Value, value: &Value) -> Result<(), Error> {
    if schema.is_null() {
        return Ok(());
    }
    let valid = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        _ => true,
    };
    if !valid {
        return Err(Code::Invalid.into());
    }
    if let Some(values) = schema["enum"].as_array()
        && !values.contains(value)
    {
        return Err(Code::Invalid.into());
    }
    if let Some(choices) = schema["anyOf"].as_array()
        && !choices.iter().any(|s| validate_shape(s, value).is_ok())
    {
        return Err(Code::Invalid.into());
    }
    if let Some(choices) = schema["oneOf"].as_array()
        && choices
            .iter()
            .filter(|s| validate_shape(s, value).is_ok())
            .count()
            != 1
    {
        return Err(Code::Invalid.into());
    }
    if let Some(number) = value.as_f64()
        && (schema["minimum"].as_f64().is_some_and(|min| number < min)
            || schema["maximum"].as_f64().is_some_and(|max| number > max))
    {
        return Err(Code::Invalid.into());
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required {
                if !object.contains_key(key.as_str().ok_or(Code::Corrupt)?) {
                    return Err(Code::Invalid.into());
                }
            }
        }
        for (key, value) in object {
            let child = &schema["properties"][key];
            if child.is_null() && schema["additionalProperties"] == false {
                return Err(Code::Invalid.into());
            }
            validate_shape(child, value)?;
        }
    }
    if let Some(items) = value.as_array() {
        for item in items {
            validate_shape(&schema["items"], item)?;
        }
    }
    Ok(())
}

pub fn descriptor_for(name: &str) -> Result<ab_catalog::OperationDescriptor, Error> {
    let descriptors = operations().map_err(|_| Error::from(Code::Corrupt))?;
    #[cfg(feature = "lab-harness")]
    let descriptors = {
        let mut ds = descriptors;
        if matches!(name, "test.echo" | "test.sleep" | "test.blob") {
            let mut d = ds
                .iter()
                .find(|d| d.name == "helper.version")
                .ok_or(Code::Corrupt)?
                .clone();
            d.name = name.into();
            d.scope = "reminders:read".into();
            ds.push(d);
        }
        ds
    };
    descriptors
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| Code::Unsupported.into())
}
