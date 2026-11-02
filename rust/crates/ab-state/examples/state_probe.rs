use ab_state::{Cursor, ReadQuery, Replica, State, Visibility};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
fn invoke(state: &mut State<Replica>, q: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let v: Visibility = serde_json::from_value(q["visibility"].clone()).unwrap_or_default();
    match q["op"].as_str().unwrap_or("") {
        "restrict" => {
            state.restrict(&v)?;
            Ok(json!({}))
        }
        "references" => Ok(serde_json::to_value(state.blob_references()?)?),
        "head" => Ok(serde_json::to_value(state.head()?)?),
        "read" => Ok(serde_json::to_value(state.read(
            &v,
            &serde_json::from_value::<ReadQuery>(q["query"].clone())?,
        )?)?),
        "model" => {
            Ok(state.read_model(&v, q["operation"].as_str().ok_or("operation")?, &q["query"])?)
        }
        "apply" => Ok(serde_json::to_value(
            state.apply_batch(&serde_json::from_value(q["batch"].clone())?)?,
        )?),
        "discard" => {
            state.discard_capture()?;
            Ok(json!({}))
        }
        "snapshot" => {
            let begin = serde_json::from_value(q["begin"].clone())?;
            let mut i = state.begin_install(
                &begin,
                q["epoch"].as_str().ok_or("epoch")?,
                q["revision"].as_u64().ok_or("revision")?,
            )?;
            for page in q["pages"].as_array().ok_or("pages")? {
                state.snapshot_page(&mut i, &serde_json::from_value(page.clone())?)?;
            }
            Ok(serde_json::to_value(state.install_snapshot(i)?)?)
        }
        "changes" => {
            let c: Cursor = serde_json::from_value(q["cursor"].clone())?;
            let mut i = state.changes(v, c)?;
            Ok(serde_json::to_value(state.next_changes(&mut i)?)?)
        }
        "digest" => {
            let result = state.read(
                &v,
                &ReadQuery {
                    limit: 10000,
                    ..ReadQuery::default()
                },
            )?;
            Ok(json!(ab_state::digest(&result.records)))
        }
        _ => Err("operation".into()),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let mut state = State::<Replica>::open(home)?;
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let q: Value = serde_json::from_str(&line?)?;
        let result = match invoke(&mut state, &q) {
            Ok(v) => v,
            Err(e) => json!({"error":e.to_string()}),
        };
        stdout.write_all(&ab_wire::gojson::record_data(
            &serde_json::to_vec(&result)?,
            ab_wire::gojson::EscapeMode::Html,
        )?)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
