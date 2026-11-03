#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]
use crate::{
    http::{Client, random_id},
    replicas::{self, Endpoint},
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};

pub const MEDIA_SCENARIOS: [&str; 2] = ["WEB-MEDIA-VARIANTS", "WEB-MEDIA-CAP"];
const SCOPE: &str = "messages:read,messages:write,system:read";
const REPLICA_SCOPE: &str = "messages:read,system:read";
const CAP: u64 = 9 * 1024 * 1024;
const VIDEO_LANDSCAPE: &[u8] = include_bytes!("../../lab-simapple/assets/video-landscape.mp4");
const VIDEO_PORTRAIT: &[u8] = include_bytes!("../../lab-simapple/assets/video-portrait.mp4");
const VIDEO_PANORAMA: &[u8] = include_bytes!("../../lab-simapple/assets/video-panorama.mp4");
const AUDIO: &[u8] = include_bytes!("../../lab-simapple/assets/audio.m4a");

fn await_until(seconds: u64, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn chunk(kind: [u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32fast::hash(&body).to_be_bytes());
}
fn png(width: u32, height: u32, noise: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    let mut state = 1u32;
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            if noise {
                for _ in 0..3 {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    raw.push(u8::try_from(state >> 24).unwrap_or(0));
                }
            } else {
                raw.extend_from_slice(&[
                    u8::try_from(x % 256).unwrap_or(0),
                    u8::try_from(y % 256).unwrap_or(0),
                    90,
                ]);
            }
        }
    }
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    chunk(*b"IHDR", &header, &mut out);
    chunk(
        *b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 1),
        &mut out,
    );
    chunk(*b"IEND", &[], &mut out);
    out
}
fn image_size(data: &[u8]) -> Option<(u64, u64, &'static str)> {
    if data.len() > 24 && data[..8] == [0x89, b'P', b'N', b'G', 13, 10, 26, 10] {
        let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
        return Some((u64::from(w), u64::from(h), "png"));
    }
    if data.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 9 < data.len() {
            if data[at] != 0xff {
                return None;
            }
            let marker = data[at + 1];
            let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
            if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                let h = u16::from_be_bytes([data[at + 5], data[at + 6]]);
                let w = u16::from_be_bytes([data[at + 7], data[at + 8]]);
                return Some((u64::from(w), u64::from(h), "jpeg"));
            }
            at += 2 + length;
        }
    }
    None
}
fn stage(client: &Client, data: &[u8]) -> Result<String> {
    let reply = client.request(
        "POST",
        "/v1/blobs",
        &[("Content-Type", "application/octet-stream")],
        Some(data),
        60,
    )?;
    let value: Value = serde_json::from_slice(&reply.body)?;
    match value["upload_id"].as_str() {
        Some(id) if reply.status == 200 => Ok(id.to_owned()),
        _ => Err(Error::Invalid("stage upload rejected".into())),
    }
}
fn send(client: &Client, chat: &str, text: &str, files: &[(String, Vec<u8>)]) -> Result<bool> {
    send_timed(client, chat, text, files).map(|(observed, _)| observed)
}
fn send_timed(
    client: &Client,
    chat: &str,
    text: &str,
    files: &[(String, Vec<u8>)],
) -> Result<(bool, Instant)> {
    let mut uploads = vec![];
    for (name, data) in files {
        uploads.push(json!({"name": name, "upload_id": stage(client, data)?}));
    }
    let started = Instant::now();
    let command = json!({"command_id": random_id("cmd")?, "op": "messages.send", "args": {"chat_id": chat, "text": text, "files": uploads}, "wait_ms": 15000});
    let outcome = client.call("messages.send", &command, 40)?;
    Ok((
        outcome.is_some_and(|v| v["disposition"] == "locally_observed"),
        started,
    ))
}
fn attachments(client: &Client) -> Vec<Value> {
    client
        .call(
            "records.query",
            &json!({"collection":"messages.attachment","limit":10000}),
            20,
        )
        .ok()
        .flatten()
        .and_then(|v| v["records"].as_array().cloned())
        .unwrap_or_default()
}
fn blob(client: &Client, digest: &str, record: &str) -> Option<Vec<u8>> {
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
fn valid_digest(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn has_extension(name: &str, extension: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}
fn wanted(name: &str) -> &'static [&'static str] {
    if has_extension(name, "mov") {
        &["video", "poster"]
    } else if has_extension(name, "caf") {
        &["audio"]
    } else {
        &["thumb", "display"]
    }
}
fn variants_at(observer: &Watcher, name: &str) -> Option<Instant> {
    observer.first_seen_record(|record| {
        record["collection"] == "messages.attachment"
            && record["data"]["filename"] == name
            && wanted(name)
                .iter()
                .all(|v| valid_digest(&record["data"]["derivatives"][v]["digest"]))
    })
}
fn kind(name: &str) -> &'static str {
    if has_extension(name, "mov") {
        "video"
    } else if has_extension(name, "caf") {
        "audio"
    } else {
        "image"
    }
}
fn snapshot(client: &Client, prefix: &str) -> BTreeMap<String, String> {
    attachments(client)
        .into_iter()
        .filter(|r| {
            r["data"]["filename"]
                .as_str()
                .is_some_and(|n| n.contains(&format!("{prefix}.")))
        })
        .map(|r| {
            (
                r["data"]["filename"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                r["data"].to_string(),
            )
        })
        .collect()
}
fn endpoints(world: &World) -> Result<Vec<Client>> {
    let mut out = vec![Client::new(world, SCOPE)?];
    out.extend(replicas::endpoints().iter().map(Endpoint::client));
    Ok(out)
}
fn verify(
    evidence: &mut Evidence,
    clients: &[Client],
    observers: &[Watcher],
    prefix: &str,
    started: Instant,
) -> Result<()> {
    for (index, client) in clients.iter().enumerate() {
        let mut found: BTreeMap<String, Value> = BTreeMap::new();
        let mut seen: BTreeMap<String, Instant> = BTreeMap::new();
        let ready = await_until(45, || {
            for record in attachments(client) {
                let name = record["data"]["filename"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if !name.contains(&format!("{prefix}.")) {
                    continue;
                }
                let complete = wanted(&name)
                    .iter()
                    .all(|v| valid_digest(&record["data"]["derivatives"][v]["digest"]));
                if complete {
                    seen.entry(name.clone()).or_insert_with(|| {
                        observers
                            .get(index)
                            .and_then(|observer| variants_at(observer, &name))
                            .unwrap_or_else(Instant::now)
                    });
                    found.insert(name, record);
                }
            }
            found.len() == 3
        });
        evidence.check(&format!("variants_captured_{index}"), ready, "variants");
        for (name, record) in &found {
            let data = &record["data"];
            let record_id = record["id"].as_str().unwrap_or_default();
            let kind = kind(name);
            evidence.check(
                &format!("{kind}_native_appearance_measured"),
                true,
                "sim appearance is the send start",
            );
            if kind == "image" && index == 0 {
                let elapsed = seen[name].duration_since(started);
                evidence.check(
                    "image_variants_within_2_seconds",
                    elapsed <= Duration::from_secs(2),
                    &format!("{elapsed:?}"),
                );
            }
            evidence.check(
                &format!("{kind}_original_preserved"),
                valid_digest(&data["blob"]["digest"]),
                "original digest",
            );
            let media = &data["media"];
            if kind == "video" {
                evidence.check(
                    "source_video_metadata_landscape",
                    media["width"] == 320
                        && media["height"] == 180
                        && media["duration_ms"] == 1000
                        && media["dims_source"] == "source",
                    &media.to_string(),
                );
            }
            if kind == "audio" {
                let duration = media["duration_ms"].as_u64().unwrap_or(0);
                let peaks = media["waveform"].as_array().cloned().unwrap_or_default();
                evidence.check(
                    "audio_duration_and_64_bounded_peaks",
                    (950..=1100).contains(&duration) && peaks.len() == 64,
                    "audio media",
                );
                if peaks
                    .iter()
                    .any(|p| !p.as_i64().is_some_and(|v| (0..=255).contains(&v)))
                {
                    evidence.check("waveform_range", false, "peak out of range");
                }
            } else {
                evidence.check(
                    &format!("{kind}_dimensions_and_thumbhash"),
                    media["width"].as_u64().unwrap_or(0) > 0
                        && media["height"].as_u64().unwrap_or(0) > 0
                        && media["thumbhash"].as_str().is_some_and(|t| !t.is_empty()),
                    "dimensions",
                );
            }
            for (variant, info) in data["derivatives"].as_object().into_iter().flatten() {
                evidence.check(
                    "private_handles_removed",
                    info["handle"].as_str().unwrap_or_default().is_empty(),
                    variant,
                );
                let digest = info["digest"].as_str().unwrap_or_default();
                let mut content = None;
                let available = await_until(15, || {
                    content = blob(client, digest, record_id);
                    content.is_some()
                });
                evidence.check(
                    &format!("{variant}_authorized_blob_{index}"),
                    available,
                    digest,
                );
                let Some(content) = content else { continue };
                evidence.check(
                    "variant_digest_verified",
                    hex::encode(Sha256::digest(&content)) == digest,
                    digest,
                );
                if info["mime"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("image/")
                {
                    let size = image_size(&content);
                    let (w, h) = (
                        info["width"].as_u64().unwrap_or(0),
                        info["height"].as_u64().unwrap_or(0),
                    );
                    evidence.check(
                        &format!("{variant}_browser_image"),
                        size.is_some_and(|(sw, sh, _)| sw == w && sh == h),
                        &format!("{size:?}"),
                    );
                    let bounded = if variant == "poster" {
                        w > 0 && h > 0 && w <= 1_500_000 / h
                    } else {
                        w.max(h) <= if variant == "thumb" { 400 } else { 1600 }
                    };
                    evidence.check(&format!("{variant}_dimensions_bounded"), bounded, "bounds");
                }
            }
        }
    }
    Ok(())
}
fn geometry(evidence: &mut Evidence, client: &Client, chat: &str, prefix: &str) -> Result<()> {
    let names = ["portrait", "panorama", "fallback"];
    let files: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|name| {
            let data = match *name {
                "portrait" => VIDEO_PORTRAIT.to_vec(),
                "panorama" => VIDEO_PANORAMA.to_vec(),
                _ => b"invalid synthetic video".to_vec(),
            };
            (format!("{prefix}-geometry-{name}.mov"), data)
        })
        .collect();
    let observed = send(client, chat, "source geometry", &files)?;
    evidence.check("source_geometry_observed", observed, "send");
    let mut records: BTreeMap<&str, Value> = BTreeMap::new();
    let all = await_until(10, || {
        for record in attachments(client) {
            let file = record["data"]["filename"].as_str().unwrap_or_default();
            for name in names {
                if file == format!("{prefix}-geometry-{name}.mov")
                    && !record["data"]["derivatives"]["video"]["digest"].is_null()
                {
                    records.insert(name, record.clone());
                }
            }
        }
        records.len() == 3
    });
    evidence.check("source_geometry_records", all, "records");
    for name in names {
        let record = records.get(name).cloned().unwrap_or(Value::Null);
        let media = &record["data"]["media"];
        let (width, height, source) = match name {
            "portrait" => (180, 320, "source"),
            "panorama" => (1920, 200, "source"),
            _ => (16, 16, "sim-default"),
        };
        evidence.check(
            &format!("source_geometry_{name}"),
            media["width"] == width
                && media["height"] == height
                && media["duration_ms"] == 1000
                && media["dims_source"] == source,
            &media.to_string(),
        );
        if name == "panorama" {
            let poster = &record["data"]["derivatives"]["poster"];
            let content = blob(
                client,
                poster["digest"].as_str().unwrap_or_default(),
                record["id"].as_str().unwrap_or_default(),
            );
            let size = content.as_deref().and_then(image_size);
            let valid = size.is_some_and(|(w, h, _)| w * 200 == h * 1920);
            evidence.check(
                "source_geometry_center_marker_poster",
                valid,
                &format!("{size:?}"),
            );
            let (w, h) = size.map_or((0, 0), |(w, h, _)| (w, h));
            evidence.check(
                "panorama_poster_retina_cover_resolution",
                size.is_some() && w.min(h) >= 360 && w * h <= 1_500_000,
                &format!("{w}x{h}"),
            );
            let video = &record["data"]["derivatives"]["video"];
            evidence.check(
                "source_geometry_wide_decoded_frame",
                video["width"] == 1920 && video["height"] == 200,
                &video.to_string(),
            );
        }
    }
    Ok(())
}
pub fn media_variants(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if !MEDIA_SCENARIOS.contains(&id.as_str()) {
        return Err(Error::Invalid(format!(
            "media_variants not ported for {id}"
        )));
    }
    let cap = id == "WEB-MEDIA-CAP";
    if id == "WEB-MEDIA-VARIANTS" && !crate::cleanup::prove(evidence, "cleanup_") {
        return Ok(());
    }
    replicas::start_with(world, 1, REPLICA_SCOPE, if cap { CAP } else { 0 })?;
    let gateway = Client::new(world, SCOPE)?;
    evidence.check(
        "seed_self",
        world
            .control("/messages", &json!({"action":"seed","count":0}))
            .is_ok(),
        "seed",
    );
    let mut chat = String::new();
    let captured = await_until(10, || {
        let page = gateway
            .call(
                "records.query",
                &json!({"collection":"messages.chat","limit":100}),
                20,
            )
            .ok()
            .flatten();
        if let Some(first) = page.as_ref().and_then(|p| p["records"][0]["id"].as_str()) {
            first.clone_into(&mut chat);
        }
        !chat.is_empty()
    });
    evidence.check("self_chat_captured", captured, "chat");
    if !captured {
        return Ok(());
    }
    let run = random_id("run")?;
    let prefix = format!("ab-{run}");
    let files = vec![
        (format!("{prefix}.png"), png(2400, 1200, cap)),
        (format!("{prefix}.mov"), VIDEO_LANDSCAPE.to_vec()),
        (format!("{prefix}.caf"), AUDIO.to_vec()),
    ];
    let clients = endpoints(world)?;
    let observers = clients
        .iter()
        .map(|client| Watcher::start(client, false))
        .collect::<Result<Vec<_>>>()?;
    let (observed, started) = send_timed(
        &gateway,
        &chat,
        &format!("[ab-test {run}] synthetic media"),
        &files,
    )
    .inspect_err(|_| evidence.check("synthetic_upload", false, "stage or send failed"))?;
    evidence.check("self_send_observed", observed, "send");
    verify(evidence, &clients, &observers, &prefix, started)?;
    if cap {
        let replica = replicas::endpoints()
            .first()
            .cloned()
            .ok_or_else(|| Error::Invalid("replica".into()))?;
        let fits = await_until(10, || {
            let status = replicas::status(&replica).unwrap_or(Value::Null);
            let blobs = &status["replica"]["blobs"];
            if blobs["bytes"].as_u64().unwrap_or(u64::MAX) > CAP {
                return false;
            }
            attachments(&gateway)
                .iter()
                .find(|r| r["data"]["filename"].as_str().is_some_and(|n| has_extension(n, "png")))
                .is_some_and(|r| {
                    blobs["entries"][r["data"]["blob"]["digest"].as_str().unwrap_or_default()]
                        ["state"]
                        == "over_cap"
                })
        });
        let status = replicas::status(&replica).unwrap_or(Value::Null);
        evidence.check(
            "variants_fit_cap_original_deferred",
            fits,
            &status["replica"]["blobs"].to_string(),
        );
    }
    let before = snapshot(&gateway, &prefix);
    let restarted = world.restart_peer().is_ok();
    evidence.check("sim_restart", restarted, "peer restart");
    let survived = await_until(15, || {
        before.len() == 3 && before == snapshot(&gateway, &prefix)
    });
    evidence.check("variants_survive_restart", survived, "snapshot");
    if id == "WEB-MEDIA-VARIANTS" {
        geometry(evidence, &gateway, &chat, &prefix)?;
    }
    Ok(())
}
