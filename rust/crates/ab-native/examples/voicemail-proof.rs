use ab_native::{capture::Context, telephony::VoicemailReader};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

fn proof() -> Result<Value, Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [store, staging] = arguments.as_slice() else {
        return Err("arguments".into());
    };
    let mut reader = VoicemailReader::open(Path::new(staging))?;
    let context = Context {
        observed_at: "2026-01-01T00:00:00.000Z".into(),
        checkpoint: "proof".into(),
        scan_id: "proof".into(),
    };
    let pending = reader.scan(Path::new(store), &context, false, 100_000)?;
    let observations = pending
        .batch
        .observations
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = json!({
        "messages": observations.len(),
        "transcripts": observations.iter().filter(|row| row["data"]["transcript_state"] == "present").count(),
        "audio": observations.iter().filter(|row| row["data"]["audio_state"] == "present").count(),
        "fetched_sha256": null,
    });
    if let Some((expected, handle)) = pending.batch.blob_handles.first_key_value() {
        let mut hash = Sha256::new();
        let mut offset = 0;
        loop {
            let chunk = reader.read_blob(handle, offset, 131_072)?;
            let bytes = STANDARD.decode(chunk["bytes"].as_str().ok_or("chunk")?)?;
            offset += u64::try_from(bytes.len())?;
            hash.update(bytes);
            if chunk["eof"] == true {
                break;
            }
        }
        let digest = format!("{:x}", hash.finalize());
        if digest != *expected {
            return Err("digest mismatch".into());
        }
        output["fetched_sha256"] = json!(digest);
    }
    Ok(output)
}
fn main() {
    if let Ok(output) = proof() {
        println!("{output}");
    } else {
        println!("{{\"status\":\"proof_failed\"}}");
        std::process::exit(1);
    }
}
