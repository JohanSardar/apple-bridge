use ab_state::{Replica, State, Visibility};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::time::Instant;
const BASE32: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
fn id(prefix: &str, n: usize) -> String {
    let mut body = [b'0'; 26];
    let mut rest = n;
    for slot in body.iter_mut().rev() {
        *slot = BASE32[rest % 32];
        rest /= 32;
    }
    format!("{prefix}_{}", String::from_utf8_lossy(&body))
}
fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let count: usize = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let state = State::<Replica>::open(&home)?;
    let db = Connection::open(std::path::Path::new(&home).join("state.db"))?;
    let calendar = id("cal", 1);
    let put = |collection: &str, record_id: &str, data: Value| -> Result<(), Box<dyn std::error::Error>> {
        let payload = json!({
            "collection": collection, "id": record_id, "version": 1, "source": "calendar",
            "observed_at": "2026-05-01T00:00:00Z", "deleted": false,
            "coverage": {"fields": "complete", "missing": []}, "data": data
        });
        db.execute(
            "INSERT INTO records(collection,id,version,source,deleted,payload,seq,observed_at) VALUES(?,?,1,'calendar',0,?,1,'2026-05-01T00:00:00Z')",
            rusqlite::params![collection, record_id, serde_json::to_vec(&payload)?],
        )?;
        Ok(())
    };
    db.execute_batch("BEGIN")?;
    put("calendar.calendar", &calendar, json!({"title": "Bridge Test range perf", "writable": true, "color": "#007aff", "account": "demo"}))?;
    for index in 0..count {
        let day = 1 + index % 28;
        let hour = 8 + (index / 28) % 8;
        let minute = (index % 4) * 15;
        let end_minute = minute + 30;
        let (end_hour, end_minute) = (hour + end_minute / 60, end_minute % 60);
        put("calendar.event", &id("evt", index + 1), json!({
            "title": format!("Bridge Test {index:04}"),
            "start": format!("2026-05-{day:02}T{hour:02}:{minute:02}:00Z"),
            "end": format!("2026-05-{day:02}T{end_hour:02}:{end_minute:02}:00Z"),
            "all_day": false, "calendar_id": calendar
        }))?;
    }
    db.execute_batch("COMMIT")?;
    let visibility: Visibility = [("calendar".to_owned(), true)].into_iter().collect();
    let query = json!({"start": "2026-05-01T00:00:00Z", "end": "2026-06-01T00:00:00Z", "time_zone": "America/New_York"});
    let mut run = |label: &str, f: &mut dyn FnMut() -> Result<usize, Box<dyn std::error::Error>>| -> Result<(), Box<dyn std::error::Error>> {
        let mut values = Vec::new();
        let mut size = 0;
        for _ in 0..120 {
            let start = Instant::now();
            size = f()?;
            values.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let p50 = percentile(&mut values, 0.5);
        let p95 = percentile(&mut values, 0.95);
        println!("{label:24} p50={p50:.3}ms p95={p95:.3}ms size={size}");
        Ok(())
    };
    run("read_model", &mut || {
        let value = state.read_model(&visibility, "calendar.range", &query)?;
        Ok(value["events"].as_array().map_or(0, Vec::len))
    })?;
    let value = state.read_model(&visibility, "calendar.range", &query)?;
    run("to_vec", &mut || Ok(serde_json::to_vec(&value)?.len()))?;
    let raw = serde_json::to_vec(&value)?;
    run("gojson", &mut || {
        Ok(ab_wire::gojson::record_data(&raw, ab_wire::gojson::EscapeMode::Html)?.len())
    })?;
    run("from_slice_value", &mut || {
        Ok(serde_json::from_slice::<Value>(&raw)?.as_object().map_or(0, |o| o.len()))
    })?;
    let plan: Vec<String> = db
        .prepare("EXPLAIN QUERY PLAN SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0 AND coalesce(json_extract(payload,'$.data.all_day'),0)=0 AND julianday(json_extract(payload,'$.data.start'))<=julianday(?) AND julianday(json_extract(payload,'$.data.end'))>=julianday(?)")?
        .query_map(["2026-06-01T00:00:00Z", "2026-05-01T00:00:00Z"], |r| r.get::<_, String>(3))?
        .collect::<Result<_, _>>()?;
    println!("plan: {plan:?}");
    Ok(())
}
