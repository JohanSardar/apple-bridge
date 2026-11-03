#![forbid(unsafe_code)]

use ab_native::usage::{Context, Reader};
use ab_wire::gojson::{self, EscapeMode};
use std::{
    io::{BufRead, Write},
    path::Path,
};

fn scan(
    reader: &mut Reader,
    root: &Path,
    live: bool,
    frames: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let context: Context = serde_json::from_slice(&std::fs::read(root.join("context.json"))?)?;
    let scan = reader.scan(root, &context, live)?;
    let mut output = std::io::stdout().lock();
    if frames {
        write_json(&mut output, &serde_json::to_value(&scan)?)?;
    } else {
        for row in &scan.observations {
            let mut value = serde_json::to_value(row)?;
            value["source"] = serde_json::json!(scan.source);
            value["generation"] = serde_json::json!(scan.generation);
            value["checkpoint"] = serde_json::json!(scan.checkpoint);
            write_json(&mut output, &value)?;
        }
        let mut checkpoint = serde_json::to_value(&scan)?;
        checkpoint
            .as_object_mut()
            .ok_or("invalid checkpoint")?
            .remove("observations");
        checkpoint["kind"] = serde_json::json!("checkpoint");
        write_json(&mut output, &checkpoint)?;
    }
    output.flush()?;
    reader.commit(scan)?;
    Ok(())
}
fn write_json(
    output: &mut impl Write,
    value: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = gojson::record_data(&serde_json::to_vec(value)?, EscapeMode::Html)?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [source, directory, options @ ..] = args.as_slice() else {
        return Err("usage: fixture-reader usage FIXTURE_DIRECTORY [--interactive]".into());
    };
    if source != "usage"
        || options
            .iter()
            .any(|o| o != "--interactive" && o != "--frames")
    {
        return Err("unsupported fixture reader or option".into());
    }
    let root = Path::new(directory);
    let mut reader = Reader::default();
    let frames = options.iter().any(|o| o == "--frames");
    if options.iter().any(|o| o == "--interactive") {
        for (index, line) in std::io::stdin().lock().lines().enumerate() {
            let line = line?;
            if line == "scan" {
                scan(&mut reader, root, index != 0, frames)?;
            } else {
                return Err("expected scan".into());
            }
        }
    } else {
        scan(&mut reader, root, false, frames)?;
    }
    Ok(())
}
