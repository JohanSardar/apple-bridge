#![forbid(unsafe_code)]

use ab_native::usage::{Context, Reader};
use std::fmt::Write as _;
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
    let bytes = ab_native::native_json::record_data(value)?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [source, directory, options @ ..] = args.as_slice() else {
        return Err(
            "usage: fixture-reader SOURCE FIXTURE_DIRECTORY [--interactive] [--frames]".into(),
        );
    };
    if ![
        "usage",
        "safari",
        "calls",
        "voicemail",
        "voice-memos",
        "mail",
    ]
    .contains(&source.as_str())
        || options
            .iter()
            .any(|o| o != "--interactive" && o != "--frames")
    {
        return Err("unsupported fixture reader or option".into());
    }
    let root = Path::new(directory);
    if source == "safari" {
        return safari(root, options);
    }
    if ["calls", "voicemail"].contains(&source.as_str()) {
        return telephony(root, options, source);
    }
    if source == "voice-memos" {
        return voice_memos(root, options);
    }
    if source == "mail" {
        return mail(root, options);
    }
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

fn safari(root: &Path, options: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use ab_native::safari::{Kind, ReadContext, Reader};
    let cache_path = root.join("rust-cache.json");
    let mut reader = match std::fs::read(&cache_path) {
        Ok(bytes) => Reader::resume(serde_json::from_slice(&bytes)?)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Reader::default(),
        Err(error) => return Err(error.into()),
    };
    let mut ordinal = 0;
    let frames = options.iter().any(|o| o == "--frames");
    let mut poll = |kinds: &[Kind]| -> Result<(), Box<dyn std::error::Error>> {
        let mut emitted = false;
        for kind in kinds {
            loop {
                let mut context: ReadContext =
                    serde_json::from_slice(&std::fs::read(root.join("context.json"))?)?;
                write!(context.capture.checkpoint, "-{ordinal}")?;
                write!(context.capture.scan_id, "-{ordinal}")?;
                let Ok(pending) = reader.poll(root, *kind, &context) else {
                    let kind = match kind {
                        Kind::History => "history",
                        Kind::Bookmarks => "bookmarks",
                        Kind::Cloud => "cloud",
                    };
                    let mut out = std::io::stdout().lock();
                    write_json(
                        &mut out,
                        &serde_json::json!({"kind":"source_state","source":"safari","state":"degraded","coverage":{"fields":"partial"},"capabilities":[],"reason":format!("Safari capture unavailable: {kind}")}),
                    )?;
                    out.flush()?;
                    emitted = true;
                    break;
                };
                let Some(pending) = pending else {
                    break;
                };
                let batch = &pending.batch;
                let mut out = std::io::stdout().lock();
                if frames {
                    write_json(&mut out, &serde_json::to_value(batch)?)?;
                } else {
                    for row in &batch.observations {
                        let mut value = serde_json::to_value(row)?;
                        value["source"] = serde_json::json!(batch.source);
                        value["generation"] = serde_json::json!(batch.generation);
                        value["checkpoint"] = serde_json::json!(batch.checkpoint);
                        write_json(&mut out, &value)?;
                    }
                    let mut value = serde_json::to_value(batch)?;
                    value
                        .as_object_mut()
                        .ok_or("invalid frame")?
                        .remove("observations");
                    value["kind"] = serde_json::json!("checkpoint");
                    write_json(&mut out, &value)?;
                }
                out.flush()?;
                let again = pending.reconcile_again;
                reader.commit(pending)?;
                std::fs::write(
                    &cache_path,
                    serde_json::to_vec(&reader.cache(&context.generation))?,
                )?;
                ordinal += 1;
                emitted = true;
                if !again {
                    break;
                }
            }
        }
        if !emitted {
            let mut out = std::io::stdout().lock();
            write_json(&mut out, &serde_json::json!({"kind":"idle"}))?;
            out.flush()?;
        }
        Ok(())
    };
    if options.iter().any(|o| o == "--interactive") {
        for line in std::io::stdin().lock().lines() {
            match line?.as_str() {
                "scan" => poll(&[Kind::History, Kind::Bookmarks, Kind::Cloud])?,
                "history" => poll(&[Kind::History])?,
                "bookmarks" => poll(&[Kind::Bookmarks])?,
                "cloud" => poll(&[Kind::Cloud])?,
                _ => return Err("expected scan, history, bookmarks or cloud".into()),
            }
        }
    } else {
        poll(&[Kind::History, Kind::Bookmarks, Kind::Cloud])?;
    }
    Ok(())
}

fn emit_capture(
    batch: &ab_native::capture::Batch,
    frames: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = std::io::stdout().lock();
    if frames {
        write_json(&mut output, &serde_json::to_value(batch)?)?;
    } else {
        for row in &batch.observations {
            let mut value = serde_json::to_value(row)?;
            value["source"] = serde_json::json!(batch.source);
            value["generation"] = serde_json::json!(batch.generation);
            value["checkpoint"] = serde_json::json!(batch.checkpoint);
            write_json(&mut output, &value)?;
        }
        let mut checkpoint = serde_json::to_value(batch)?;
        checkpoint
            .as_object_mut()
            .ok_or("invalid checkpoint")?
            .remove("observations");
        checkpoint["kind"] = serde_json::json!("checkpoint");
        write_json(&mut output, &checkpoint)?;
    }
    output.flush()?;
    Ok(())
}
fn telephony(
    root: &Path,
    options: &[String],
    kind: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use ab_native::{
        capture::{Context, State},
        telephony,
    };
    let mut state = State::default();
    let frames = options.iter().any(|o| o == "--frames");
    let mut scan = |limit: usize, live: bool| -> Result<(), Box<dyn std::error::Error>> {
        let context: Context = serde_json::from_slice(&std::fs::read(root.join("context.json"))?)?;
        let scan = if kind == "calls" {
            telephony::calls
        } else {
            telephony::voicemail
        };
        if let Ok(pending) = scan(&root.join("store.db"), &state, &context, live, limit) {
            emit_capture(&pending.batch, frames)?;
            state.commit(pending)?;
        } else {
            let mut output = std::io::stdout().lock();
            write_json(&mut output, &serde_json::json!({"kind":"failed_scan"}))?;
            output.flush()?;
        }
        Ok(())
    };
    if options.iter().any(|o| o == "--interactive") {
        for (index, line) in std::io::stdin().lock().lines().enumerate() {
            scan(line?.parse().unwrap_or(100_000), index > 0)?;
        }
    } else {
        scan(100_000, false)?;
    }
    Ok(())
}

fn voice_memos(root: &Path, options: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use ab_native::{capture::Context, voice_memo::Reader};
    let mut reader = Reader::open(&root.join("rust-blobs"))?;
    let mut index = 0;
    let frames = options.iter().any(|o| o == "--frames");
    let mut run = |line: &str| -> Result<(), Box<dyn std::error::Error>> {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if let ["blob", handle, offset, length] = parts.as_slice() {
            let result = offset
                .parse()
                .ok()
                .zip(length.parse().ok())
                .and_then(|(offset, length)| reader.read_blob(handle, offset, length).ok());
            let mut output = std::io::stdout().lock();
            write_json(
                &mut output,
                &result.unwrap_or_else(|| serde_json::json!({"kind":"failed_blob"})),
            )?;
            output.flush()?;
            return Ok(());
        }
        let context: Context = serde_json::from_slice(&std::fs::read(root.join("context.json"))?)?;
        if let Ok(pending) = reader.scan(
            &root.join("store.db"),
            &context,
            index > 0,
            line.parse().unwrap_or(100_000),
        ) {
            emit_capture(&pending.batch, frames)?;
            reader.commit(pending)?;
        } else {
            let mut output = std::io::stdout().lock();
            write_json(&mut output, &serde_json::json!({"kind":"failed_scan"}))?;
            output.flush()?;
        }
        index += 1;
        Ok(())
    };
    if options.iter().any(|o| o == "--interactive") {
        for line in std::io::stdin().lock().lines() {
            run(&line?)?;
        }
    } else {
        run("scan")?;
    }
    Ok(())
}

fn mail(root: &Path, options: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use ab_native::{capture::Context, mail::Reader, mail_body};
    let mut reader = Reader::default();
    let frames = options.iter().any(|o| o == "--frames");
    let mut run = |line: &str| -> Result<(), Box<dyn std::error::Error>> {
        if let Some(args) = line.strip_prefix("body ") {
            let args: serde_json::Value = serde_json::from_str(args)?;
            let value = mail_body::read(&args, &root.join("bodies"))
                .unwrap_or_else(|_| serde_json::json!({"kind":"failed_body"}));
            let mut output = std::io::stdout().lock();
            write_json(&mut output, &value)?;
            output.flush()?;
            return Ok(());
        }
        let context: Context = serde_json::from_slice(&std::fs::read(root.join("context.json"))?)?;
        if let Ok(result) = reader.poll(
            &root.join("store.db"),
            &context,
            line.parse().unwrap_or(100_000),
        ) {
            if let Some(pending) = result {
                emit_capture(&pending.batch, frames)?;
                reader.commit(pending)?;
            } else {
                let mut output = std::io::stdout().lock();
                write_json(&mut output, &serde_json::json!({"kind":"idle"}))?;
                output.flush()?;
            }
        } else {
            let mut output = std::io::stdout().lock();
            write_json(&mut output, &serde_json::json!({"kind":"failed_scan"}))?;
            output.flush()?;
        }
        Ok(())
    };
    if options.iter().any(|o| o == "--interactive") {
        for line in std::io::stdin().lock().lines() {
            run(&line?)?;
        }
    } else {
        run("scan")?;
    }
    Ok(())
}
