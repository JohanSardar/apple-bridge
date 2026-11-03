#[derive(Debug, Default)]
struct Ledger(std::sync::Mutex<BTreeMap<String, ab_operations::local::CommandRecord>>);
impl ab_operations::local::GatewayState for Ledger {
    fn admission_era(&self) -> Result<String, ab_operations::Error> {
        Ok("probe-era".into())
    }
    fn command(
        &self,
        id: &str,
    ) -> Result<ab_operations::local::CommandRecord, ab_operations::Error> {
        self.0
            .lock()
            .map_err(|_| Code::Unavailable)?
            .get(id)
            .cloned()
            .ok_or_else(|| Code::NotFound.into())
    }
    fn commands(&self) -> Result<Vec<ab_operations::local::CommandRecord>, ab_operations::Error> {
        Ok(self
            .0
            .lock()
            .map_err(|_| Code::Unavailable)?
            .values()
            .cloned()
            .collect())
    }
    fn transition(
        &self,
        from: Option<ab_operations::domain::CommandState>,
        mut record: ab_operations::local::CommandRecord,
    ) -> Result<ab_operations::local::CommandRecord, ab_operations::Error> {
        let mut rows = self.0.lock().map_err(|_| Code::Unavailable)?;
        if rows.get(&record.command.id).map(|r| r.state) != from {
            return Err(Code::Conflict.into());
        }
        record
            .history
            .push(serde_json::json!({"state":record.state,"disposition":record.disposition}));
        rows.insert(record.command.id.clone(), record.clone());
        Ok(record)
    }
    fn update_parts(
        &self,
        record: ab_operations::local::CommandRecord,
    ) -> Result<ab_operations::local::CommandRecord, ab_operations::Error> {
        self.0
            .lock()
            .map_err(|_| Code::Unavailable)?
            .insert(record.command.id.clone(), record.clone());
        Ok(record)
    }
    fn head(&self) -> Result<Cursor, ab_operations::Error> {
        Ok(Cursor {
            epoch: "probe".into(),
            subscription_revision: 1,
            sequence: 0,
        })
    }
    fn observed_record_id(
        &self,
        _: &str,
        key: &str,
        _: &Cursor,
    ) -> Result<String, ab_operations::Error> {
        Ok(format!("synthetic_{key}"))
    }
}
#[derive(Debug)]
struct Identity;
fn principal(id: &str) -> ab_operations::Principal {
    ab_operations::Principal {
        id: id.into(),
        kind: if id.starts_with("agent:") {
            "agent"
        } else {
            "device"
        }
        .into(),
        grants: vec!["admin".into()],
        level: if id.starts_with("agent:") {
            "approve"
        } else {
            "full"
        }
        .into(),
        resource: String::new(),
        remote: false,
    }
}
impl ab_operations::Identity for Identity {
    fn authenticate(&self, token: &str) -> Result<ab_operations::Principal, ab_operations::Error> {
        Ok(principal(token))
    }
    fn current(
        &self,
        p: &ab_operations::Principal,
    ) -> Result<ab_operations::Principal, ab_operations::Error> {
        Ok(principal(&p.id))
    }
    fn sign(
        &self,
        _: &ab_operations::Principal,
        _: &str,
        _: &str,
    ) -> Result<String, ab_operations::Error> {
        Err(Code::Unsupported.into())
    }
}
#[derive(Debug)]
struct Codec;
impl ab_operations::Codec for Codec {
    fn json(&self, bytes: &[u8]) -> Result<Vec<u8>, ab_operations::Error> {
        ab_wire::gojson::legacy_raw(bytes, ab_wire::gojson::EscapeMode::Html)
            .map_err(|_| Code::Corrupt.into())
    }
    fn command_hash(&self, c: &ab_operations::Command) -> Result<String, ab_operations::Error> {
        ab_wire::gojson::CommandHash {
            op: &c.op,
            args: c.args.get().as_bytes(),
            version: c.expected_version,
            expires: ab_wire::gojson::GoTime::parse(&c.expires_at).map_err(|_| Code::Invalid)?,
            era: &c.admission_era,
        }
        .digest()
        .map_err(|_| Code::Invalid.into())
    }
    fn fingerprint(&self, bytes: &[u8]) -> String {
        hex::encode(bytes)
    }
}
#[derive(Debug)]
struct Actions;
impl ab_operations::local::ActionResolver for Actions {
    fn resolve(
        &self,
        _: &ab_operations::Principal,
        c: &ab_operations::Command,
    ) -> Result<ab_operations::local::PreparedAction, ab_operations::Error> {
        let args: serde_json::Value =
            serde_json::from_str(c.args.get()).map_err(|_| Code::Invalid)?;
        let mut action = serde_json::json!({"op":c.op,"args":args,"origin":c.origin});
        if c.op == "messages.send" {
            action["messages"] = args;
            action["args"] = serde_json::json!({});
        }
        if c.expected_version > 0 {
            return Err(Code::Conflict.into());
        }
        Ok(ab_operations::local::PreparedAction {
            action: serde_json::value::to_raw_value(&action).map_err(|_| Code::Corrupt)?,
            expected_state: None,
            collection: if c.op == "messages.send" {
                "messages.message"
            } else {
                "reminders.list"
            }
            .into(),
            serialization_key: "probe".into(),
        })
    }
}
