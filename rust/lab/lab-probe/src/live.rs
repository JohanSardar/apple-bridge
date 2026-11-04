use crate::{
    Estimate, Local, Result, Session,
    actions::{common, sleep},
    clock::to_ms,
    failed,
    local::Handler,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub type Seen = Arc<Mutex<HashMap<String, i128>>>;

fn effect_time(effect: &Value, name: &str) -> i128 {
    i128::from(effect[name].as_i64().unwrap_or_default())
}

pub fn observe(seen: &Seen, collection: &str, field: &str, prefix: &str) -> Handler {
    let seen = seen.clone();
    let (collection, field, prefix) = (collection.to_owned(), field.to_owned(), prefix.to_owned());
    Box::new(move |at, batch| {
        let Ok(mut map) = seen.lock() else {
            return;
        };
        for change in batch["changes"].as_array().into_iter().flatten() {
            let record = &change["record"];
            if !collection.is_empty() && record["collection"] != collection.as_str() {
                continue;
            }
            if let Some(value) = record["data"][field.as_str()].as_str()
                && value.starts_with(&prefix)
            {
                map.entry(value.to_owned()).or_insert(at);
            }
        }
    })
}

fn latencies(effects: &[Value], seen: &Seen, clock: &Estimate) -> Result<Option<Vec<f64>>> {
    let map = seen.lock().map_err(|_| crate::Error::Invalid("lock"))?;
    let mut values = Vec::new();
    for effect in effects {
        let title = effect["title"].as_str().unwrap_or_default();
        let Some(&at) = map.get(title) else {
            continue;
        };
        let (wall, started) = (
            effect_time(effect, "wall_ns"),
            effect_time(effect, "started_wall_ns"),
        );
        let latency = at - wall + clock.offset_ns;
        let before_save = started > 0
            && started <= wall
            && at + clock.offset_ns >= started - clock.uncertainty_ns;
        if wall <= 0 || latency < -clock.uncertainty_ns && !before_save {
            return failed("clock agreement failed");
        }
        values.push(to_ms(latency).max(0.0));
    }
    Ok((values.len() == effects.len()).then_some(values))
}

pub fn run<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let input = common(&value)?;
    let local = Local::new(&input)?;
    let field = match value["Field"].as_str() {
        Some(field) if !field.is_empty() => field.to_owned(),
        _ => "title".to_owned(),
    };
    let prefix = value["Prefix"].as_str().unwrap_or_default().to_owned();
    let head = local.op("records.query", &json!({"collection": "", "limit": 1}))?;
    let seen: Seen = Seen::default();
    let watch = local.watch(
        &head["cursor"],
        observe(&seen, &input.collection, &field, &prefix),
    )?;
    let clock = Estimate::load(std::path::Path::new(&input.home))?;
    session.ready()?;
    let effects = session.input::<Vec<Value>>()?;
    if effects.is_empty() {
        return failed("no effects");
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(values) = latencies(&effects, &seen, &clock)? {
            return session.emit(
                &json!({"latencies_ms": values, "clock_uncertainty_ms": clock.uncertainty_ms()}),
            );
        }
        if watch.finished() {
            return failed("live disconnected before convergence");
        }
        if Instant::now() >= deadline {
            return failed("context deadline exceeded");
        }
        sleep(10);
    }
}
