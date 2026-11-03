use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, id, json, lock, now, observation,
    send, string,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write as _, io::Read, path::Path, thread};

const LIMIT: u64 = 8 << 20;
const FIXTURE_PDF: &str = "f71a9207e47bd6526a9a5a346541df4eb84de43649bcb11fce1ba7aa971b07cc";
const MISSING_CALLBACK: &str = "[ab-test missing callback]";
const CAPTURE_OPS: [&str; 5] = [
    "capture.screenshot",
    "capture.camera_photo",
    "capture.record_audio",
    "mac.screen_record",
    "print.document",
];
const OCR_FIXTURES: [(&str, &str, &str); 3] = [
    (
        "cd72477f797d20b7b3445289d36ddaac2be46d511dbb390612748d2462475e68",
        "Bridge fixture",
        "en",
    ),
    (
        "7feee5a2404aaed713290b4fe33bf8320898dcebab51e100e0ed06ac59358fea",
        "Bonjour le monde",
        "fr",
    ),
    (
        "53acf3e11bfeaf8d400cff84cf93861b6592da9573b206ab025e8b1a82063739",
        "Guten Morgen",
        "de",
    ),
];

pub(super) fn capabilities() -> [(&'static str, Vec<&'static str>); 4] {
    [
        ("pdf", vec!["pdf.annotate"]),
        ("speech", vec!["speech.say"]),
        ("print", vec!["print.document"]),
        (
            "capture",
            vec![
                "capture.play_sound",
                "capture.screenshot",
                "capture.camera_photo",
                "capture.record_audio",
            ],
        ),
    ]
}

fn reply(id: &Value, outcome: std::result::Result<Value, &'static str>) -> Value {
    match outcome {
        Ok(result) => json!({"t":"QueryResult","id":id,"result_json":result,"blobs":[]}),
        Err(code) => json!({"t":"QueryError","id":id,"code":code}),
    }
}

fn text_field<'a>(args: &'a Value, name: &str) -> std::result::Result<&'a str, &'static str> {
    match &args[name] {
        Value::Null => Ok(""),
        Value::String(text) => Ok(text),
        _ => Err("invalid"),
    }
}

fn object<'a>(
    args: &'a Value,
    allowed: &[&str],
) -> std::result::Result<&'a serde_json::Map<String, Value>, &'static str> {
    let map = args.as_object().ok_or("invalid")?;
    if map.keys().all(|name| allowed.contains(&name.as_str())) {
        Ok(map)
    } else {
        Err("invalid")
    }
}

fn read_file(path: &str) -> std::result::Result<Vec<u8>, &'static str> {
    if !Path::new(path).is_absolute() {
        return Err("invalid");
    }
    let mut file = std::fs::File::open(path).map_err(|_| "unavailable")?;
    let meta = file.metadata().map_err(|_| "invalid")?;
    if !meta.is_file() || meta.len() > LIMIT {
        return Err("invalid");
    }
    let mut data = vec![];
    Read::by_ref(&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(|_| "invalid")?;
    Ok(data)
}

fn input(file: &str, data: &str, allow_empty: bool) -> std::result::Result<Vec<u8>, &'static str> {
    let bytes = if file.is_empty() {
        STANDARD.decode(data).map_err(|_| "invalid")?
    } else {
        if !data.is_empty() {
            return Err("invalid");
        }
        read_file(file)?
    };
    if bytes.len() as u64 > LIMIT || (bytes.is_empty() && !allow_empty) {
        return Err("invalid");
    }
    Ok(bytes)
}

fn png_valid(data: &[u8]) -> bool {
    use zune_core::bytestream::ZCursor;
    data.starts_with(b"\x89PNG\r\n\x1a\n")
        && zune_png::PngDecoder::new(ZCursor::new(data))
            .decode_headers()
            .is_ok()
}

fn ocr(args: &Value) -> std::result::Result<Value, &'static str> {
    let map = object(args, &["file", "data_base64", "languages"])?;
    let file = text_field(args, "file")?;
    let data = text_field(args, "data_base64")?;
    if let Some(languages) = map.get("languages")
        && !languages.is_null()
        && !languages
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string))
    {
        return Err("invalid");
    }
    let bytes = input(file, data, false)?;
    if !png_valid(&bytes) {
        return Err("invalid");
    }
    let digest = hex::encode(Sha256::digest(&bytes));
    let (_, text, language) = OCR_FIXTURES
        .iter()
        .find(|(known, _, _)| *known == digest)
        .ok_or("unavailable")?;
    Ok(
        json!({"text":text,"language":language,"lines":[{"text":text,"box":{"x":0,"y":0,"width":1,"height":1}}]}),
    )
}

fn known_artifact(state: &Shared, digest: &str) -> Result<bool> {
    fn walk(value: &Value, digest: &str) -> bool {
        match value {
            Value::Object(fields) => {
                ["digest", "raw_body_digest"]
                    .iter()
                    .any(|name| fields.get(*name).and_then(Value::as_str) == Some(digest))
                    || fields.values().any(|inner| walk(inner, digest))
            }
            Value::Array(items) => items.iter().any(|inner| walk(inner, digest)),
            _ => false,
        }
    }
    let s = lock(state)?;
    Ok(s.records()
        .iter()
        .any(|record| record["collection"] == "pdf.artifact" && walk(&record["data"], digest)))
}

fn pdf(state: &Shared, op: &str, args: &Value) -> Result<std::result::Result<Value, &'static str>> {
    let checked = (|| {
        object(args, &["file", "data_base64", "text"])?;
        let file = text_field(args, "file")?;
        let data = text_field(args, "data_base64")?;
        let text = text_field(args, "text")?;
        let bytes = input(file, data, true)?;
        if !bytes.starts_with(b"%PDF-") || (op == "pdf.search" && text.is_empty()) {
            return Err("invalid");
        }
        Ok((bytes, text.to_owned()))
    })();
    let (bytes, text) = match checked {
        Ok(pair) => pair,
        Err(code) => return Ok(Err(code)),
    };
    let digest = hex::encode(Sha256::digest(&bytes));
    if digest != FIXTURE_PDF && !known_artifact(state, &digest)? {
        return Ok(Err("unavailable"));
    }
    if op == "pdf.text" {
        return Ok(Ok(
            json!({"pages":[{"page":1,"text":"Bridge fixture"}],"page_count":1}),
        ));
    }
    let mut matches = vec![];
    if let Some(offset) = "bridge fixture".find(&text.to_lowercase()) {
        matches.push(json!({"page":1,"offset":offset,"length":text.len()}));
    }
    Ok(Ok(json!({"matches":matches,"page_count":1})))
}

fn wav() -> Vec<u8> {
    let mut wav = vec![0u8; 364];
    let mut put = |at: usize, bytes: &[u8]| wav[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"RIFF");
    put(4, &356u32.to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes());
    put(22, &1u16.to_le_bytes());
    put(24, &16000u32.to_le_bytes());
    put(28, &32000u32.to_le_bytes());
    put(32, &2u16.to_le_bytes());
    put(34, &16u16.to_le_bytes());
    put(36, b"data");
    put(40, &320u32.to_le_bytes());
    wav
}

fn transcribe(args: &Value) -> std::result::Result<Value, &'static str> {
    object(args, &["data_base64", "file", "language"])?;
    let data = text_field(args, "data_base64")?;
    let file = text_field(args, "file")?;
    let language = text_field(args, "language")?;
    if language.len() > 64 || data.is_empty() == file.is_empty() {
        return Err("invalid");
    }
    let bytes = input(file, data, false)?;
    if bytes != wav() {
        return Err("unavailable");
    }
    Ok(json!({
        "status":"available","text":"Bridge fixture","language":"en-US",
        "segments":[{"text":"Bridge fixture","start":0,"end":0.01}]
    }))
}

fn synthesize(
    blobs: &mut BTreeMap<String, Vec<u8>>,
    id: &Value,
    args: &Value,
) -> std::result::Result<Value, &'static str> {
    object(args, &["text"])?;
    let text = text_field(args, "text")?;
    if text.is_empty() || text.len() > 2048 {
        return Err("invalid");
    }
    let data = wav();
    let digest = hex::encode(Sha256::digest(&data));
    let size = data.len();
    blobs.insert(digest.clone(), data);
    Ok(
        json!({"t":"QueryResult","id":id,"result_json":{"media_type":"audio/wav"},"blobs":[{"digest":digest,"size":size,"handle":digest}]}),
    )
}

pub(super) fn query(
    state: &Shared,
    blobs: &mut BTreeMap<String, Vec<u8>>,
    frame: &Value,
) -> Result<Option<Value>> {
    let id = &frame["id"];
    let args = &frame["args"];
    let outcome = match frame["op"].as_str() {
        Some("vision.ocr") => ocr(args),
        Some("pdf.text" | "pdf.search") => pdf(state, frame["op"].as_str().unwrap_or(""), args)?,
        Some("audio.transcribe") => transcribe(args),
        Some("speech.synthesize") => {
            return Ok(Some(match synthesize(blobs, id, args) {
                Ok(frame) => frame,
                Err(code) => reply(id, Err(code)),
            }));
        }
        _ => return Ok(None),
    };
    Ok(Some(reply(id, outcome)))
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace(['\r', '\n'], " ")
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn annotated_pdf(annotations: &[Value]) -> Vec<u8> {
    let mut refs = String::new();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        String::new(),
        String::new(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for (index, a) in annotations.iter().enumerate() {
        let _ = write!(refs, "{} 0 R ", index + 6);
        let [x, y, w, h] = ["x", "y", "width", "height"].map(|k| a[k].as_f64().unwrap_or(0.0));
        let (kind, quad) = if a["type"] == "highlight" {
            let points = [x, y + h, x + w, y + h, x, y, x + w, y]
                .iter()
                .map(|v| number(*v))
                .collect::<Vec<_>>()
                .join(" ");
            ("Highlight", format!(" /QuadPoints [{points}]"))
        } else {
            ("Text", String::new())
        };
        objects.push(format!(
            "<< /Type /Annot /Subtype /{kind} /Rect [{} {} {} {}] /Contents ({}){quad} >>",
            number(x),
            number(y),
            number(x + w),
            number(y + h),
            escape(a["text"].as_str().unwrap_or(""))
        ));
    }
    let content = "BT /F1 12 Tf 20 100 Td (Bridge fixture) Tj ET\n";
    objects[3] = format!(
        "<< /Length {} >>\nstream\n{content}endstream",
        content.len()
    );
    objects[2] = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [{refs}] >>"
    );
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = vec![];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = write!(out, "{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let start = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for offset in &offsets {
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }
    let _ = write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
        offsets.len() + 1
    );
    out.into_bytes()
}

fn pdf_input(args: &Value) -> std::result::Result<Vec<u8>, &'static str> {
    let file = text_field(args, "file")?;
    let data = text_field(args, "data_base64")?;
    if file.is_empty() {
        let bytes = STANDARD.decode(data).map_err(|_| "invalid")?;
        return Ok(bytes);
    }
    let mut handle = std::fs::File::open(file).map_err(|_| "unavailable")?;
    let meta = handle.metadata().map_err(|_| "invalid")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > LIMIT {
        return Err("invalid");
    }
    let mut bytes = vec![];
    Read::by_ref(&mut handle)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid")?;
    Ok(bytes)
}

fn annotations_fit(args: &Value) -> bool {
    args["annotations"].as_array().is_some_and(|items| {
        items.iter().all(|a| {
            let [x, y, w, h] = ["x", "y", "width", "height"].map(|k| a[k].as_f64().unwrap_or(0.0));
            a["page"].as_i64() == Some(1) && x + w <= 600.0 && y + h <= 800.0
        })
    })
}

impl Session<'_> {
    fn vision_operation(&self, frame: &Value) -> String {
        match frame["t"].as_str() {
            Some("Prepare") => string(&frame["action"], "op"),
            Some("Dispatch") => self
                .tickets
                .get(&string(frame, "ticket"))
                .map(|ticket| string(&ticket.prepare["action"], "op"))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub(super) fn vision_command(&mut self, state: &Shared, frame: &Value) -> Result<bool> {
        let op = self.vision_operation(frame);
        let playback = op == "speech.say" || op == "capture.play_sound";
        let capture = CAPTURE_OPS.contains(&op.as_str());
        if !(playback || capture || op == "pdf.annotate") {
            return Ok(false);
        }
        if frame["t"] == "Prepare" {
            self.vision_prepare(frame, if capture { "playback" } else { route(&op) })?;
            return Ok(true);
        }
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return Ok(false);
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            self.reject(frame, "expired")?;
            return Ok(true);
        }
        lock(state)?
            .guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        if playback {
            self.playback(state, frame, &ticket)?;
        } else if capture {
            self.capture(state, frame, &ticket, &op)?;
        } else {
            self.annotate(state, frame, &ticket)?;
        }
        Ok(true)
    }

    fn vision_prepare(&mut self, frame: &Value, route: &str) -> Result<()> {
        let ticket = id("ticket")?;
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: Value::Null,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":frame["command_id"],"ticket":ticket,"route":route,"fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    fn finish(&mut self, state: &Shared, outcome: &Value) -> Result<()> {
        lock(state)?.guard(&json!({"command_id":outcome["command_id"],"outcome":outcome}))?;
        send(&mut self.stream, outcome)?;
        Ok(())
    }

    fn playback(&mut self, state: &Shared, frame: &Value, ticket: &Ticket) -> Result<()> {
        let args = &ticket.prepare["action"]["args"];
        let callback = args["text"].as_str() != Some(MISSING_CALLBACK);
        if !callback {
            thread::sleep(Duration::from_millis(50));
        }
        let disposition = if callback {
            "locally_observed"
        } else {
            "indeterminate"
        };
        let outcome = json!({"t":"Outcome","command_id":frame["command_id"],"disposition":disposition,"evidence":{"finished_callback":callback,"observed":callback,"prepared":{"request_hash":ticket.prepare["request_hash"]}}});
        self.finish(state, &outcome)
    }

    fn capture(&mut self, state: &Shared, frame: &Value, ticket: &Ticket, op: &str) -> Result<()> {
        let data = format!("[ab-test capture artifact] {op}").into_bytes();
        let digest = hex::encode(Sha256::digest(&data));
        let size = data.len();
        self.blobs.insert(digest.clone(), data);
        let outcome = json!({"t":"Outcome","command_id":frame["command_id"],"disposition":"locally_observed","evidence":{"digest":digest,"size":size,"handle":digest,"observed":true,"prepared":{"request_hash":ticket.prepare["request_hash"]}}});
        self.finish(state, &outcome)
    }

    fn annotate(&mut self, state: &Shared, frame: &Value, ticket: &Ticket) -> Result<()> {
        let command = string(frame, "command_id");
        let args = &ticket.prepare["action"]["args"];
        let failed = json!({"t":"Outcome","command_id":command,"disposition":"failed_no_effect"});
        let input = pdf_input(args);
        let valid = input
            .as_ref()
            .is_ok_and(|bytes| hex::encode(Sha256::digest(bytes)) == FIXTURE_PDF);
        if !valid || !annotations_fit(args) {
            send(&mut self.stream, &failed)?;
            return Ok(());
        }
        let annotations = args["annotations"].as_array().cloned().unwrap_or_default();
        let data = annotated_pdf(&annotations);
        let digest = hex::encode(Sha256::digest(&data));
        let size = data.len();
        self.blobs.insert(digest.clone(), data);
        let key = id("pdf")?;
        let record = observation(
            "pdf.artifact",
            &key,
            json!({"blob":{"digest":digest,"size":size,"state":"present"},"annotation_count":annotations.len()}),
        );
        let mut s = lock(state)?;
        s.put(record.clone());
        s.save()?;
        drop(s);
        self.emit(
            state,
            json!({"t":"Capture","source":"pdf","observations":[record],"blob_handles":{digest.clone():digest},"scan_id":id("scan")?,"scope":["pdf.artifact"],"scan_started_at":now(),"completion_evidence":"complete"}),
        )?;
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"native_key":key,"observed":true,"fingerprint":ticket.prepare["request_hash"],"capture_checkpoint":self.checkpoints.get("pdf"),"annotation_count":annotations.len()}});
        self.finish(state, &outcome)
    }
}

fn route(op: &str) -> &'static str {
    if op == "pdf.annotate" {
        "sim-pdf"
    } else {
        "playback"
    }
}
