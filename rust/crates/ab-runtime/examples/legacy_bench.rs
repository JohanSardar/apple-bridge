use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("path")?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut values = Vec::new();
    let mut size = 0;
    for _ in 0..120 {
        let start = Instant::now();
        let raw = ab_operations::legacy::read_model("calendar.range", &value)?;
        let bytes = ab_wire::gojson::legacy_raw(raw.get().as_bytes(), ab_wire::gojson::EscapeMode::Html)?;
        size = bytes.len();
        values.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    values.sort_by(f64::total_cmp);
    println!("legacy+encode p50={:.3} p95={:.3} size={size}", values[60], values[113]);
    Ok(())
}
