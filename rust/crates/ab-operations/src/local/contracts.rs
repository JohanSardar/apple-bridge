use super::{
    Arc, BTreeMap, Code, Command, Cursor, Debug, Deserialize, Disposition, Error, IpcFrame,
    Principal, RawValue, Serialize, State, Value, target_collection,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<CommandPart>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub upload_references: BTreeMap<String, ab_state::BlobReference>,
    pub command: Command,
    pub principal: String,
    pub source: String,
    pub request_hash: String,
    pub state: State,
    pub admission_era: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
    pub commit_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_record_ids: Vec<String>,
    pub cursor: Cursor,
    pub history: Vec<ab_state::CommandEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}
#[derive(Debug)]
pub struct PreparedAction {
    pub action: Box<RawValue>,
    pub expected_state: Option<Box<RawValue>>,
    pub collection: String,
    pub serialization_key: String,
}
pub trait GatewayState: Debug + Send + Sync {
    fn admission_era(&self) -> Result<String, Error>;
    fn serialization_key(
        &self,
        _principal: &Principal,
        _command: &Command,
        key: String,
    ) -> Result<String, Error> {
        Ok(key)
    }
    fn command(&self, id: &str) -> Result<CommandRecord, Error>;
    fn commands(&self) -> Result<Vec<CommandRecord>, Error>;
    fn transition(
        &self,
        from: Option<State>,
        record: CommandRecord,
    ) -> Result<CommandRecord, Error>;
    fn update_parts(&self, record: CommandRecord) -> Result<CommandRecord, Error>;
    fn head(&self) -> Result<Cursor, Error>;
    fn observed_record_id(
        &self,
        collection: &str,
        key: &str,
        cursor: &Cursor,
    ) -> Result<String, Error>;
}
#[derive(Debug, Clone)]
pub struct HelperBlob {
    pub digest: String,
    pub size: u64,
    pub handle: String,
}
#[derive(Debug, Clone)]
pub enum OutcomeEvidence {
    CaptureBound,
    Standalone,
    Artifact(HelperBlob),
    Unproven,
}
pub type ActionLease = Arc<dyn Debug + Send + Sync>;
pub type PreparedWithLease = (PreparedAction, Option<ActionLease>);
#[derive(Debug, Default)]
pub struct BorrowedUploads {
    pub references: BTreeMap<String, ab_state::BlobReference>,
    pub lease: Option<ActionLease>,
}
pub trait ActionResolver: Debug + Send + Sync {
    fn serialization_key(
        &self,
        _principal: &Principal,
        command: &Command,
    ) -> Result<String, Error> {
        crate::command_contract::serialization_key(command)
    }

    fn borrow_uploads(
        &self,
        _principal: &Principal,
        command: &Command,
    ) -> Result<BorrowedUploads, Error> {
        if command.op.starts_with("messages.") {
            let args: Value =
                serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
            if args.as_object().is_some_and(|object| {
                object.iter().any(|(key, value)| {
                    key.eq_ignore_ascii_case("files")
                        && value.as_array().is_some_and(|files| !files.is_empty())
                })
            }) {
                return Err(Code::Unavailable.into());
            }
        }
        Ok(BorrowedUploads::default())
    }

    fn prepare(
        &self,
        principal: &Principal,
        command: &Command,
    ) -> Result<PreparedWithLease, Error> {
        self.resolve(principal, command).map(|plan| (plan, None))
    }

    fn collection(&self, command: &Command) -> Result<String, Error> {
        Ok(target_collection(&command.op))
    }
    fn validate(&self, command: &Command) -> Result<(), Error> {
        crate::command_contract::validate(command).map(|_| ())
    }
    fn evidence(
        &self,
        command: &Command,
        hash: &str,
        frame: &IpcFrame,
    ) -> Result<OutcomeEvidence, Error> {
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let evidence: Value = frame
            .evidence
            .as_ref()
            .and_then(|value| serde_json::from_str(value.get()).ok())
            .unwrap_or(Value::Null);
        let observed =
            frame.disposition == "locally_observed" || frame.disposition == "absent_confirmed";
        if crate::outcome::playback(&command.op)
            && matches!(
                frame.disposition.as_str(),
                "locally_observed" | "delivered" | "delivery_failed"
            )
            && (!observed || !crate::outcome::valid(&command.op, &args, &evidence, hash))
        {
            return Ok(OutcomeEvidence::Unproven);
        }
        if observed && !crate::outcome::valid(&command.op, &args, &evidence, hash) {
            return Ok(OutcomeEvidence::Unproven);
        }
        if observed && crate::outcome::capture(&command.op) {
            return Ok(OutcomeEvidence::Artifact(HelperBlob {
                digest: evidence["digest"].as_str().ok_or(Code::Corrupt)?.into(),
                size: evidence["size"].as_u64().ok_or(Code::Corrupt)?,
                handle: evidence["handle"].as_str().ok_or(Code::Corrupt)?.into(),
            }));
        }
        Ok(if crate::outcome::requires_capture(&command.op) {
            OutcomeEvidence::CaptureBound
        } else {
            OutcomeEvidence::Standalone
        })
    }

    fn resolve(&self, principal: &Principal, command: &Command) -> Result<PreparedAction, Error>;
    fn conflict(
        &self,
        _principal: &Principal,
        _command: &Command,
        frame: &IpcFrame,
    ) -> Result<Option<Error>, Error> {
        frame
            .error
            .as_ref()
            .map(|value| serde_json::from_str(value.get()).map_err(|_| Code::Corrupt.into()))
            .transpose()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPart {
    pub index: usize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub upload_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub record_id: String,
    pub disposition: String,
}
