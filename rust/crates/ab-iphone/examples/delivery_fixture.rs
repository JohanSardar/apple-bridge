use ab_iphone::{
    Error,
    delivery::{Delivery, Gateway, HealthBatch, LocationRecord, QueuedCommand},
};
use ab_trust::{
    Authority,
    domain::{CommandState, Disposition},
    iphone::Request,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Arc, Mutex},
};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    rows: Vec<QueuedCommand>,
    evidence: Vec<Value>,
    streams: BTreeMap<String, (u64, String)>,
    locations: BTreeMap<String, Value>,
    denied: bool,
}
#[derive(Debug, Clone)]
struct Adapter(Arc<Mutex<Store>>);
impl Adapter {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, Error> {
        self.0.lock().map_err(|_| Error::Unavailable)
    }
}
impl Gateway for Adapter {
    fn commands(&mut self) -> Result<Vec<QueuedCommand>, Error> {
        Ok(self.lock()?.rows.clone())
    }
    fn command(&mut self, id: &str) -> Result<QueuedCommand, Error> {
        self.lock()?
            .rows
            .iter()
            .find(|r| {
                serde_json::from_str::<Value>(r.command.get()).is_ok_and(|v| v["command_id"] == id)
            })
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn transition(
        &mut self,
        prior: &QueuedCommand,
        state: CommandState,
        _disposition: Option<Disposition>,
        evidence: Option<&RawValue>,
    ) -> Result<QueuedCommand, Error> {
        let mut store = self.lock()?;
        let row = store
            .rows
            .iter_mut()
            .find(|r| r.command.get() == prior.command.get())
            .ok_or(Error::NotFound)?;
        if row.state != prior.state {
            return Err(Error::Conflict);
        }
        row.state = state;
        let out = row.clone();
        if let Some(raw) = evidence {
            store.evidence.push(serde_json::from_str(raw.get())?);
        }
        Ok(out)
    }
    fn authorize(&mut self, _command: &QueuedCommand) -> Result<(), Error> {
        if self.lock()?.denied {
            Err(Error::Unauthorized)
        } else {
            Ok(())
        }
    }
    fn decide(&mut self, id: &str, hash: &str, approve: bool) -> Result<(), Error> {
        let row = self.command(id)?;
        if row.request_hash != hash || row.state != CommandState::AwaitingApproval {
            return Err(Error::Unauthorized);
        }
        self.transition(
            &row,
            if approve {
                CommandState::Admitted
            } else {
                CommandState::RejectedByOwner
            },
            None,
            None,
        )?;
        Ok(())
    }
    fn ingest_health(
        &mut self,
        device: &str,
        batch: &HealthBatch,
        digest: &str,
    ) -> Result<(), Error> {
        let mut store = self.lock()?;
        let cursor = store
            .streams
            .entry(format!("{device}/{}", batch.stream))
            .or_default();
        let prior = ab_iphone::delivery::HealthCursor {
            sequence: cursor.0,
            digest: cursor.1.clone(),
        };
        if let Some(next) = batch.advance(Some(&prior))? {
            *cursor = (next.sequence, next.digest);
        }
        let _ = digest;
        Ok(())
    }
    fn ingest_locations(&mut self, device: &str, records: &[LocationRecord]) -> Result<(), Error> {
        let mut store = self.lock()?;
        for record in records {
            store.locations.insert(
                format!("{device}:{}", record.id),
                serde_json::to_value(record)?,
            );
        }
        Ok(())
    }
}
fn run(input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let authority = Arc::new(Authority::open(input["home"].as_str().ok_or("home")?)?);
    match input["op"].as_str().ok_or("op")? {
        "health-digest" => {
            let batch: HealthBatch = serde_json::from_value(input["batch"].clone())?;
            batch.validate()?;
            Ok(
                json!({"digest":batch.digest()?, "raw":String::from_utf8(ab_wire::gojson::legacy_json(&batch, ab_wire::gojson::EscapeMode::Html)?)?}),
            )
        }
        "live" => {
            let p = authority.principal(input["id"].as_str().ok_or("id")?)?;
            let (token, aps) = ab_iphone::delivery::live_payload(
                &p,
                &input["command"],
                input["now"].as_i64().ok_or("now")?,
            )?;
            Ok(json!({"token":token,"raw":aps.get()}))
        }
        "list" => Ok(serde_json::to_value(authority.iphones()?)?),
        "revoke" => {
            authority.revoke_iphone(input["id"].as_str().ok_or("id")?)?;
            Ok(json!({"revoked":true}))
        }
        "delivery" => {
            let store = Arc::new(Mutex::new(serde_json::from_value::<Store>(
                input["store"].clone(),
            )?));
            let delivery = Delivery::new(authority, Adapter(store.clone()));
            let now = input["now"].as_i64().ok_or("now")?;
            let result = if input["path"] == "sweep" {
                delivery.sweep(now).map(|()| json!({"swept":true}))
            } else {
                let request: Request = serde_json::from_value(input["request"].clone())?;
                delivery
                    .handle(input["path"].as_str().ok_or("path")?, &request, now)
                    .and_then(|r| Ok(serde_json::from_str(r.get())?))
            };
            let result = result.unwrap_or_else(|e| json!({"error":e.to_string()}));
            Ok(json!({"result": result, "store":*store.lock().map_err(|_| "poisoned")?}))
        }
        _ => Err("op".into()),
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw)?;
    if input["op"] == "serve" {
        use std::io::Write;
        let authority = Arc::new(Authority::open(input["home"].as_str().ok_or("home")?)?);
        let store = Arc::new(Mutex::new(serde_json::from_value::<Store>(
            input["store"].clone(),
        )?));
        let delivery = Arc::new(Delivery::new(authority, Adapter(store)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        println!("http://{}", listener.local_addr()?);
        std::io::stdout().flush()?;
        axum::serve(listener, delivery.router()).await?;
        return Ok(());
    }
    let output = run(&input).unwrap_or_else(|e| json!({"error":e.to_string()}));
    println!("{output}");
    Ok(())
}
