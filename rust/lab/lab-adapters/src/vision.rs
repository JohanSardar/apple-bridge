use crate::{
    assertions::await_until,
    http::{Client, random_id},
    mac::command_call,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub const SIM_SCENARIOS: [&str; 5] = ["AUD-SIM", "PDF-ANNOTATE", "PDF-TEXT", "VIS-OCR", "CAP-SIM"];
pub const REAL_SCENARIOS: [&str; 3] = ["AUD-TRANSCRIBE", "SPEECH-SAY", "VIS-IDLE"];
const VISION_SCOPE: &str = "vision:read,pdf:read,system:read";
const AUDIO_SCOPE: &str = "speech:read,speech:write,capture:write,audio:read,system:read";
const CAPTURE_SCOPE: &str =
    "calendar:read,reminders:read,system:read,capture:write,mac:write,print:write";
const FIXTURES: &str = "lab/fixtures/vision";

pub fn owns(id: &str) -> bool {
    SIM_SCENARIOS.contains(&id) || REAL_SCENARIOS.contains(&id)
}

fn query_body(args: &Value, deadline: u64) -> Result<Value> {
    Ok(json!({"id": random_id("qry")?, "args": args, "deadline_ms": deadline}))
}
fn ask(client: &Client, op: &str, args: &Value, deadline: u64) -> Result<Option<(u16, Value)>> {
    client.call_raw(op, &query_body(args, deadline)?, 60)
}
fn good(reply: Option<&(u16, Value)>) -> Option<&Value> {
    reply
        .filter(|(status, value)| *status == 200 && value["error"].is_null())
        .map(|(_, value)| value)
}
fn rejected_with(reply: Option<&(u16, Value)>, code: &str) -> bool {
    reply.is_some_and(|(_, value)| value["error"]["code"] == code)
}
fn fixture(name: &str) -> std::io::Result<Vec<u8>> {
    fs::read(Path::new(FIXTURES).join(name))
}
fn absolute(name: &str) -> Option<String> {
    fs::canonicalize(Path::new(FIXTURES).join(name))
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
}
fn blob_get(client: &Client, digest: &str, record: &str) -> Option<Vec<u8>> {
    let reply = client
        .request(
            "GET",
            &format!("/v1/blobs/{digest}?record={record}"),
            &[],
            None,
            30,
        )
        .ok()?;
    (reply.status == 200).then_some(reply.body)
}

fn ready(
    world: &World,
    scope: &str,
    scope_name: &str,
    evidence: &mut Evidence,
) -> Result<Option<Client>> {
    let client = Client::new(world, scope)?;
    evidence.check(scope_name, true, "scoped token created");
    let up = await_until(15, || {
        query_body(&json!({}), 1000)
            .ok()
            .and_then(|body| client.call("helper.version", &body, 20).ok().flatten())
            .is_some()
    });
    evidence.check("helper_ready", up, "helper version answers");
    Ok(up.then_some(client))
}

fn cpu_seconds(pid: &str) -> f64 {
    let Ok(output) = Command::new("ps").args(["-o", "time=", "-p", pid]).output() else {
        return -1.0;
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let parts: Vec<&str> = text.split(':').collect();
    if !output.status.success() || parts.len() != 2 {
        return -1.0;
    }
    match (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
        (Ok(minutes), Ok(seconds)) => minutes * 60.0 + seconds,
        _ => -1.0,
    }
}

fn idle(world: &World, evidence: &mut Evidence) {
    let pid = fs::read_to_string(world.gateway_home.join("helper").join("helper.pid"));
    evidence.check("helper_pid_available", pid.is_ok(), "helper pid file read");
    let Ok(pid) = pid else { return };
    let pid = pid.trim().to_owned();
    let before = cpu_seconds(&pid);
    let started = Instant::now();
    thread::sleep(Duration::from_secs(300));
    let after = cpu_seconds(&pid);
    let elapsed = started.elapsed().as_secs_f64();
    evidence.check(
        "helper_cpu_under_two_percent",
        before >= 0.0 && after >= before && (after - before) / elapsed < 0.02,
        &format!("helper cpu {:.3}%", 100.0 * (after - before) / elapsed),
    );
}

#[allow(clippy::too_many_lines)]
fn annotate(world: &World, evidence: &mut Evidence) -> Result<()> {
    let data = fixture("fixture.pdf");
    evidence.check("fixture_read", data.is_ok(), "fixture pdf read");
    let data = data.unwrap_or_default();
    let encoded = STANDARD.encode(&data);
    let args = json!({"data_base64": encoded, "annotations": [
        {"page":1,"type":"note","text":"[ab-test synthetic]","x":10,"y":10,"width":20,"height":20},
        {"page":1,"type":"highlight","text":"","x":40,"y":40,"width":50,"height":15}
    ]});
    let client = Client::new(world, "pdf:read,pdf:write")?;
    evidence.check("annotation_scopes", true, "scoped token created");
    let empty = json!({"data_base64": encoded, "annotations": []});
    let bad = client.call_raw(
        "pdf.annotate",
        &crate::journey::command("pdf.annotate", &empty)?,
        40,
    )?;
    evidence.check(
        "empty_annotations_rejected",
        rejected_with(bad.as_ref(), "invalid"),
        "empty annotation list is invalid",
    );
    let result = client
        .call_raw(
            "pdf.annotate",
            &crate::journey::command("pdf.annotate", &args)?,
            40,
        )?
        .map_or(Value::Null, |(_, value)| value);
    let ids = result["observed_record_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    evidence.check(
        "annotation_observed",
        result["error"].is_null() && result["state"] == "observed" && ids.len() == 1,
        "annotation command observed one record",
    );
    if ids.len() != 1 || !result["error"].is_null() {
        return Ok(());
    }
    let record_id = ids[0].as_str().unwrap_or_default().to_owned();
    let records = client.call(
        "records.query",
        &json!({"collection":"pdf.artifact","id":record_id}),
        30,
    )?;
    let page = records
        .as_ref()
        .and_then(|value| value["records"].as_array())
        .filter(|items| items.len() == 1);
    evidence.check(
        "artifact_metadata",
        page.is_some(),
        "artifact metadata record found",
    );
    let Some(page) = page else { return Ok(()) };
    let metadata = &page[0]["data"];
    evidence.check(
        "annotation_count",
        metadata["annotation_count"] == 2,
        "two annotations counted",
    );
    let digest = metadata["blob"]["digest"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut output = None;
    let fetched = await_until(10, || {
        output = blob_get(&client, &digest, &record_id);
        output.is_some()
    });
    let output = output.unwrap_or_default();
    evidence.check(
        "new_pdf_blob_digest",
        fetched && hex::encode(Sha256::digest(&output)) == digest && output != data,
        "stored blob digest differs from the input",
    );
    let blob_args =
        json!({"blob":{"collection":"pdf.artifact","record_id":record_id,"digest":digest}});
    let extracted = ask(&client, "pdf.text", &blob_args, 10000)?;
    let pages = good(extracted.as_ref()).and_then(|value| value["result_json"]["pages"].as_array());
    evidence.check(
        "artifact_blob_input_preserves_document_text",
        pages.is_some_and(|items| {
            items.len() == 1
                && items[0]["text"]
                    .as_str()
                    .is_some_and(|t| t.trim() == "Bridge fixture")
        }),
        "artifact blob input keeps the page text",
    );
    let has = |needle: &str| output.windows(needle.len()).any(|w| w == needle.as_bytes());
    evidence.check(
        "annotation_page_geometry_preserved",
        has("/MediaBox [0 0 600 800]"),
        "media box unchanged",
    );
    evidence.check(
        "output_contains_note_and_highlight",
        has("/Subtype /Text")
            && has("/Subtype /Highlight")
            && has("/Contents ([ab-test synthetic])"),
        "output holds both annotations",
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn documents(client: &Client, id: &str, evidence: &mut Evidence) -> Result<()> {
    let pdf = id == "PDF-TEXT";
    let op = if pdf { "pdf.text" } else { "vision.ocr" };
    let bad = ask(
        client,
        op,
        &json!({"data_base64": STANDARD.encode("invalid fixture")}),
        10000,
    )?;
    evidence.check(
        "malformed_document_rejected",
        rejected_with(bad.as_ref(), "invalid"),
        "invalid document is rejected",
    );
    let (files, texts): (&[&str], &[&str]) = if pdf {
        (&["fixture.pdf"], &["Bridge fixture"])
    } else {
        (
            &["en.png", "fr.png", "de.png"],
            &["Bridge fixture", "Bonjour le monde", "Guten Morgen"],
        )
    };
    for (file, text) in files.iter().zip(texts) {
        let data = fixture(file);
        evidence.check("generated_fixture_read", data.is_ok(), "fixture read");
        let Ok(data) = data else { return Ok(()) };
        let reply = ask(
            client,
            op,
            &json!({"data_base64": STANDARD.encode(&data)}),
            30000,
        )?;
        evidence.check(
            "fixture_query_completed",
            good(reply.as_ref()).is_some(),
            "query completed",
        );
        let result = good(reply.as_ref()).map(|value| value["result_json"].clone());
        let matched = result.as_ref().is_some_and(|value| {
            if pdf {
                value["pages"]
                    .as_array()
                    .filter(|p| p.len() == 1)
                    .map_or_else(
                        || value["text"] == *text,
                        |p| p[0]["text"].as_str().is_some_and(|t| t.trim() == *text),
                    )
            } else {
                value["text"] == *text
            }
        });
        evidence.check(
            "fixture_content_matched",
            matched,
            "content matches the fixture text",
        );
        let path = absolute(file);
        let denied = ask(client, op, &json!({"file": path}), 10000)?;
        evidence.check(
            "file_requires_opt_in",
            rejected_with(denied.as_ref(), "invalid"),
            "file input without opt-in is invalid",
        );
        let blob = json!({"blob":{"query_id":"qry_missing","digest":"0".repeat(64)}});
        let unowned = ask(client, op, &blob, 10000)?;
        evidence.check(
            "unowned_blob_rejected",
            rejected_with(unowned.as_ref(), "forbidden"),
            "unowned blob is forbidden",
        );
        let file_reply = ask(
            client,
            op,
            &json!({"file": path, "allow_file": true}),
            30000,
        )?;
        let file_result = good(file_reply.as_ref()).map(|value| value["result_json"].clone());
        evidence.check(
            "file_input_matches_inline",
            path.is_some() && file_result.is_some() && file_result == result,
            "file input equals inline input",
        );
        if pdf {
            for needle in ["fixture", "missing"] {
                let search = ask(
                    client,
                    "pdf.search",
                    &json!({"data_base64": STANDARD.encode(&data), "text": needle}),
                    10000,
                )?;
                let found =
                    good(search.as_ref()).and_then(|v| v["result_json"]["matches"].as_array());
                let valid = found.is_some_and(|items| {
                    if needle == "fixture" {
                        items.len() == 1 && items[0]["page"] == 1 && items[0]["length"] == 7
                    } else {
                        items.is_empty()
                    }
                });
                evidence.check("pdf_search_match_count_and_page", valid, "search matches");
            }
        } else {
            let shaped = result.as_ref().is_some_and(|value| {
                value["lines"].as_array().is_some_and(|lines| {
                    !lines.is_empty() && lines[0]["box"].as_object().is_some_and(|b| b.len() == 4)
                }) && value["language"].as_str().is_some_and(|l| !l.is_empty())
            });
            evidence.check("ocr_shape", shaped, "lines carry boxes and a language");
        }
    }
    Ok(())
}

fn vision(world: &World, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let Some(client) = ready(world, VISION_SCOPE, "query_scopes_granted", evidence)? else {
        return Ok(());
    };
    match id.as_str() {
        "VIS-IDLE" => {
            idle(world, evidence);
            Ok(())
        }
        "PDF-ANNOTATE" => annotate(world, evidence),
        _ => documents(&client, &id, evidence),
    }
}

fn words_match(text: &str) -> f64 {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let matched = ["bridge", "fixture"]
        .iter()
        .filter(|expected| words.iter().any(|w| w == **expected))
        .count();
    f64::from(u32::try_from(matched).unwrap_or(0)) / 2.0
}

fn playback(client: &Client, id: &str, evidence: &mut Evidence) -> Result<()> {
    for op in ["speech.say", "capture.play_sound"] {
        let args = if op == "capture.play_sound" {
            json!({"name":"Tink","volume":0})
        } else {
            json!({"text":"Bridge fixture","volume":0})
        };
        let record = command_call(client, op, &args);
        evidence.check(
            "playback_finished_callback",
            record.is_some_and(|r| r["state"] == "observed"),
            "playback finished callback observed",
        );
    }
    if id == "AUD-SIM" {
        let command_id = random_id("cmd")?;
        let body = json!({"command_id": command_id, "op": "speech.say", "args": {"text":"[ab-test missing callback]","volume":0}, "wait_ms": 15000});
        let _ = client.call_raw("speech.say", &body, 40)?;
        let status = client.call("commands.status", &json!({"command_id": command_id}), 40)?;
        evidence.check(
            "missing_callback_is_indeterminate",
            status.is_some_and(|r| r["state"] == "indeterminate"),
            "missing callback leaves the command indeterminate",
        );
    }
    Ok(())
}

fn audio(world: &World, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let Some(client) = ready(world, AUDIO_SCOPE, "scopes_granted", evidence)? else {
        return Ok(());
    };
    if id == "SPEECH-SAY" || id == "AUD-SIM" {
        playback(&client, &id, evidence)?;
        if id == "SPEECH-SAY" {
            return Ok(());
        }
    }
    let synth = ask(
        &client,
        "speech.synthesize",
        &json!({"text":"Bridge fixture"}),
        60000,
    )?;
    let out = good(synth.as_ref()).cloned().unwrap_or(Value::Null);
    let blobs = out["blobs"].as_array().cloned().unwrap_or_default();
    evidence.check(
        "synthesis_completed",
        good(synth.as_ref()).is_some()
            && out["result_json"]["media_type"] == "audio/wav"
            && blobs.len() == 1
            && blobs[0]["size"].as_u64().unwrap_or(0) > 44,
        "synthesis returned one wav blob",
    );
    if blobs.len() != 1 {
        return empty_text(&client, evidence);
    }
    let digest = blobs[0]["digest"].as_str().unwrap_or_default().to_owned();
    let record = out["id"].as_str().unwrap_or_default().to_owned();
    let data = blob_get(&client, &digest, &record);
    let valid = data.as_ref().is_some_and(|bytes| {
        bytes.len() > 44
            && bytes.len() as u64 == blobs[0]["size"].as_u64().unwrap_or(0)
            && bytes.starts_with(b"RIFF")
            && hex::encode(Sha256::digest(bytes)) == digest
    });
    evidence.check(
        "wav_blob_readback_digest",
        valid,
        "wav blob matches its digest",
    );
    if (id == "AUD-TRANSCRIBE" || id == "AUD-SIM") && valid {
        let Some(bytes) = blob_get(&client, &digest, &record) else {
            evidence.check("fixture_read", false, "blob read again");
            return Ok(());
        };
        let reply = ask(
            &client,
            "audio.transcribe",
            &json!({"data_base64": STANDARD.encode(&bytes)}),
            60000,
        )?;
        let result = good(reply.as_ref()).map(|v| v["result_json"].clone());
        let accepted = result.as_ref().is_some_and(|r| {
            if r["status"] == "available" {
                let segments = r["segments"].as_array().cloned().unwrap_or_default();
                words_match(r["text"].as_str().unwrap_or("")) >= 0.8
                    && r["language"].as_str().is_some_and(|l| !l.is_empty())
                    && !segments.is_empty()
                    && segments.iter().all(|s| {
                        let (start, end) = (
                            s["start"].as_f64().unwrap_or(-1.0),
                            s["end"].as_f64().unwrap_or(-1.0),
                        );
                        start >= 0.0 && end >= start
                    })
            } else {
                r["status"] == "unavailable"
                    && r["reason"]
                        == "Speech Recognition authorization for the Helper is not_determined"
            }
        });
        evidence.check(
            "on_device_transcript_or_authorization_limit",
            accepted,
            "transcript matches or authorization limit reported",
        );
    }
    empty_text(&client, evidence)
}

fn empty_text(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let reply = ask(client, "speech.synthesize", &json!({"text":""}), 10000)?;
    evidence.check(
        "empty_text_rejected",
        rejected_with(reply.as_ref(), "invalid"),
        "empty text is invalid",
    );
    Ok(())
}

fn native_checks(evidence: &mut Evidence) -> Option<PathBuf> {
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "CaptureChecks")
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_build", ok, "CaptureChecks built");
    if !ok {
        return None;
    }
    let directory = fs::read_to_string("native/.build/native-output-1").ok();
    evidence.check(
        "native_output",
        directory.is_some(),
        "native output directory",
    );
    directory.map(|text| PathBuf::from(text.trim()))
}

fn capture(world: &World, evidence: &mut Evidence) -> Result<()> {
    let Some(directory) = native_checks(evidence) else {
        return Ok(());
    };
    let ran = Command::new(directory.join("CaptureChecks")).output();
    evidence.check(
        "provider_lifecycle_and_artifact_verification",
        ran.is_ok_and(|o| o.status.success()),
        "CaptureChecks passed",
    );
    let client = Client::new(world, CAPTURE_SCOPE)?;
    evidence.check("public_gateway_start", true, "gateway started");
    let up = await_until(5, || {
        query_body(&json!({}), 1000)
            .ok()
            .and_then(|body| client.call("helper.version", &body, 20).ok().flatten())
            .is_some()
    });
    evidence.check("public_helper_ready", up, "helper version answers");
    for protocol in ["rest", "mcp"] {
        for op in [
            "capture.screenshot",
            "capture.camera_photo",
            "capture.record_audio",
            "mac.screen_record",
            "print.document",
        ] {
            let args = match op {
                "capture.record_audio" | "mac.screen_record" => json!({"seconds":2}),
                "print.document" => {
                    let file = absolute("fixture.pdf");
                    evidence.check(
                        "print_fixture_path",
                        file.is_some(),
                        "fixture path resolved",
                    );
                    json!({"file": file, "printer": "PDF"})
                }
                _ => json!({}),
            };
            let body = crate::journey::command(op, &args)?;
            let reply = if protocol == "rest" {
                client.call_raw(op, &body, 40)?
            } else {
                client.call_mcp_raw(op, &body, 40)?
            };
            let record = reply
                .as_ref()
                .filter(|(_, value)| value["error"].is_null())
                .map_or(Value::Null, |(_, value)| value.clone());
            let observed = record["state"] == "observed";
            evidence.check(
                &format!("{protocol}_{op}_observed"),
                observed,
                "command observed",
            );
            let digest = record["evidence"]["digest"].as_str().unwrap_or_default();
            let blob = record["evidence"]["blob_id"].as_str().unwrap_or_default();
            let size = record["evidence"]["size"].as_u64().unwrap_or(0);
            let data = client
                .request(
                    "GET",
                    &format!("/v1/blobs/{digest}?record={blob}"),
                    &[],
                    None,
                    30,
                )
                .ok()
                .filter(|r| r.status == 200);
            let valid = data.is_some_and(|r| {
                r.body.len() as u64 == size && hex::encode(Sha256::digest(&r.body)) == digest
            });
            evidence.check(
                &format!("{protocol}_{op}_blob_download"),
                valid,
                "blob matches evidence",
            );
        }
    }
    Ok(())
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "CAP-SIM" => capture(world, evidence),
        "AUD-SIM" | "AUD-TRANSCRIBE" | "SPEECH-SAY" => audio(world, evidence),
        "PDF-ANNOTATE" | "PDF-TEXT" | "VIS-OCR" | "VIS-IDLE" => vision(world, evidence),
        other => Err(Error::Invalid(format!(
            "vision scenario not ported: {other}"
        ))),
    }
}
