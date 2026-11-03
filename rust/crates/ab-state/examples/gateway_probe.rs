fn command_record(raw: &str) -> Result<ab_state::CommandRecord, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        record: Box<serde_json::value::RawValue>,
    }
    serde_json::from_str(serde_json::from_str::<Envelope>(raw)?.record.get())
}
use ab_state::{Gateway, State, Visibility};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
fn invoke(
    s: &mut State<Gateway>,
    q: &Value,
    raw: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let visibility: Visibility =
        serde_json::from_value(q["visibility"].clone()).unwrap_or_default();
    Ok(match q["op"].as_str().ok_or("op")? {
        "capture" => {
            serde_json::to_value(s.commit_capture(&serde_json::from_value(q["capture"].clone())?)?)?
        }
        #[cfg(feature = "lab")]
        "max_pages" => {
            s.lab_max_pages(u32::try_from(q["pages"].as_u64().ok_or("pages")?)?)?;
            json!({})
        }
        "source" => {
            s.source_state(
                serde_json::from_value(q["source"].clone())?,
                "ready",
                serde_json::from_value(json!({"fields":"complete","missing":[]}))?,
                Some(&[]),
                "",
            )?;
            json!({})
        }
        "model" => s.read_model(
            &visibility,
            q["operation"].as_str().ok_or("operation")?,
            &q["query"],
        )?,
        "head" => serde_json::to_value(s.head()?)?,
        "era" => json!(s.admission_era()?),
        "fence" => json!(s.next_fence()?),
        "checkpoints" => serde_json::to_value(s.checkpoints()?)?,
        "command" => serde_json::to_value(s.command(q["id"].as_str().ok_or("id")?)?)?,
        "commands" => serde_json::to_value(s.commands()?)?,
        "transition" => serde_json::to_value(s.transition_command(
            serde_json::from_value(q["from"].clone())?,
            command_record(raw)?,
        )?)?,
        "parts" => serde_json::to_value(s.update_command_parts(command_record(raw)?)?)?,
        "freeze" => serde_json::to_value(
            s.freeze_snapshot(&visibility, q["revision"].as_u64().unwrap_or(1))?,
        )?,
        "close" => {
            s.close_snapshot(q["id"].as_str().ok_or("id")?)?;
            json!({})
        }
        "renew" => {
            s.renew_snapshot(q["id"].as_str().ok_or("id")?)?;
            json!({})
        }
        "prune" => {
            s.prune(q["rows"].as_u64().unwrap_or(0), std::time::Duration::ZERO)?;
            json!({})
        }
        "backup" => {
            s.backup(q["path"].as_str().ok_or("path")?)?;
            json!({})
        }
        "native" => json!(s.native_key(
            q["collection"].as_str().ok_or("collection")?,
            q["id"].as_str().ok_or("id")?
        )?),
        "read" => serde_json::to_value(
            s.read(&visibility, &serde_json::from_value(q["query"].clone())?)?,
        )?,
        _ => return Err("op".into()),
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    if let Some(backup) = std::env::args().nth(2) {
        ab_state::restore(&home, backup, false)?;
        return Ok(());
    }
    let mut s = State::<Gateway>::open(home)?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let q: Value = serde_json::from_str(&line)?;
        let result = invoke(&mut s, &q, &line).unwrap_or_else(|e| json!({"error":e.to_string()}));
        println!("{}", serde_json::to_string(&result)?);
        io::stdout().flush()?;
    }
    Ok(())
}
