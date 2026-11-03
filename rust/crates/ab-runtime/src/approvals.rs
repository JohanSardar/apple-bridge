use ab_operations::{Code, local::LocalGateway};
use ab_trust::{Error, Principal};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug)]
pub struct Approvals(pub Arc<LocalGateway>);
fn error(value: &ab_operations::Error) -> Error {
    match value.code {
        Code::Unauthorized | Code::Forbidden => Error::Unauthorized,
        Code::NotFound | Code::Invalid => Error::Invalid,
        _ => Error::Unavailable,
    }
}
impl ab_frontdoor::web::Approvals for Approvals {
    fn pending(&self) -> Result<Vec<Value>, Error> {
        self.0
            .approvals()
            .map_err(|value| error(&value))?
            .into_iter()
            .map(|record| {
                Ok(
                    json!({"command_id":record.command.id, "operation":record.command.op,
                "principal":record.principal, "arguments":"[redacted]",
                "owner_arguments":serde_json::to_string_pretty(&record.command.args)?}),
                )
            })
            .collect()
    }
    fn command(&self, id: &str) -> Result<Value, Error> {
        let row = self.0.state.command(id).map_err(|value| error(&value))?;
        let row = self
            .0
            .approvals()
            .map_err(|value| error(&value))?
            .into_iter()
            .find(|record| record.command.id == id)
            .unwrap_or(row);
        Ok(serde_json::to_value(row)?)
    }
    fn decide(&self, owner: &Principal, id: &str, approve: bool) -> Result<Value, Error> {
        let owner = ab_operations::Principal {
            id: owner.id.clone(),
            kind: owner.kind.clone(),
            grants: owner
                .grants
                .iter()
                .map(|grant| grant.as_str().to_owned())
                .collect(),
            level: owner.level.clone(),
            resource: String::new(),
            remote: owner.remote,
        };
        let result = tokio::runtime::Handle::current()
            .block_on(self.0.decide(&owner, id, approve))
            .map_err(|value| error(&value))?;
        Ok(serde_json::from_str(result.get())?)
    }
}
