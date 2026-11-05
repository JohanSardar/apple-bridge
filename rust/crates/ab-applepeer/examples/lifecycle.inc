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
        expires_at: None,
    }
}
impl ab_operations::Identity for Identity {
    fn approval_principal(&self, id: &str, _: &str, _: &str) -> Result<ab_operations::Principal, ab_operations::Error> { Ok(principal(id)) }
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

#[derive(Debug,Default)]
struct Audit(std::sync::Mutex<Vec<ab_operations::QueryAudit>>);
impl ab_operations::QueryAuditSink for Audit {
    fn record(&self,_: &ab_operations::Principal,_: &str,a: &ab_operations::QueryAudit,_: bool) -> Result<(),ab_operations::Error> {
        self.0.lock().map_err(|_|Code::Unavailable)?.push(a.clone());Ok(())
    }
}
