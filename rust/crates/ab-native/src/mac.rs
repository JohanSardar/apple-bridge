use crate::capture::{self, Context, Error, Observation, Pending, Result, ScanInput, State};
use serde_json::Value;

pub const SOURCE: &str = "mac";
pub const GENERATION: &str = "native/1";
pub const SCOPE: [&str; 5] = [
    "mac.audio_device",
    "mac.display",
    "mac.power",
    "mac.network",
    "mac.app",
];

pub fn observations(rows: &[Value], context: &Context) -> Result<Vec<Observation>> {
    rows.iter()
        .map(|row| {
            let collection = row["collection"].as_str().ok_or(Error::Corrupt)?;
            let key = row["native_key"].as_str().ok_or(Error::Corrupt)?;
            if row["kind"] != "upsert" || !SCOPE.contains(&collection) || !row["data"].is_object() {
                return Err(Error::Corrupt);
            }
            capture::upsert(
                collection,
                key,
                Some(format!("mac/{collection}/{key}")),
                row["data"].clone(),
                context,
            )
        })
        .collect()
}

pub fn prepare(state: &State, rows: &[Value], context: &Context, live: bool) -> Result<Pending> {
    state.prepare(ScanInput {
        source: SOURCE,
        generation: GENERATION,
        records: observations(rows, context)?,
        scope: SCOPE.iter().map(|scope| (*scope).into()).collect(),
        coverage: Value::Null,
        context,
        live,
    })
}
