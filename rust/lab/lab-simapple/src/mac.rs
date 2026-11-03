use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, enqueue, invalid, json, lock, now,
    send, string,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{fmt::Write as _, sync::mpsc};

pub(super) const CAPABILITIES: [&str; 22] = [
    "mac.screen_record",
    "mac.set_volume",
    "mac.set_mute",
    "mac.set_appearance",
    "mac.launch_app",
    "mac.activate_app",
    "mac.quit_app",
    "mac.keep_awake",
    "mac.clipboard_write",
    "mac.clipboard_read",
    "mac.notify",
    "mac.open_settings",
    "mac.set_wallpaper",
    "mac.lock_screen",
    "mac.sleep_display",
    "mac.restart",
    "mac.shutdown",
    "mac.network_check",
    "mac.bluetooth_devices",
    "mac.time_machine_status",
    "mac.recognize_song",
    "mac.iwork_export",
];
const COLLECTIONS: [&str; 6] = [
    "mac.app",
    "mac.audio_device",
    "mac.bluetooth_device",
    "mac.display",
    "mac.network",
    "mac.power",
];
const SONG_BLOB: &str = "YnJpZGdlLWxhYi1hdWRpbw==";

pub(super) fn control_notifications(state: &Shared, input: &Value) -> Result<Value> {
    let permission = input["permission"]
        .as_str()
        .ok_or_else(|| invalid("permission"))?;
    let mut s = lock(state)?;
    s.data["notification_permission"] = json!(permission);
    s.save()?;
    Ok(json!({"effect_done":true}))
}

pub(super) fn control_mac(
    state: &Shared,
    tx: &mpsc::Sender<super::Request>,
    input: &Value,
) -> Result<Value> {
    let collection = string(input, "collection");
    let key = string(input, "key");
    if !COLLECTIONS.contains(&collection.as_str()) || key.is_empty() || !input["data"].is_object() {
        return Err(invalid("mac record"));
    }
    let mut s = lock(state)?;
    s.data["records"][format!("{collection}/{key}")] = json!({
        "kind":"upsert",
        "collection":collection,
        "native_key":key,
        "scoped_alias":format!("mac/{collection}/{key}"),
        "data":input["data"],
        "observed_at":now()
    });
    s.save()?;
    let frame = s.scan("mac")?;
    drop(s);
    enqueue(tx, frame, true)?;
    Ok(json!({"effect_done":true}))
}

pub(super) fn query(state: &Shared, frame: &Value) -> Result<Option<Value>> {
    let id = &frame["id"];
    let args = &frame["args"];
    let result = |value: Value| json!({"t":"QueryResult","id":id,"result_json":value,"blobs":[]});
    let reply = match frame["op"].as_str() {
        Some("mac.bluetooth_devices") => result(
            json!({"devices":[{"name":"Bridge Test Headphones","connected":true,"battery_percent":75}]}),
        ),
        Some("mac.time_machine_status") => result(json!({"running":false,"phase":"idle"})),
        Some("mac.recognize_song") => {
            if args["audio_blob"] == SONG_BLOB && args.as_object().is_some_and(|m| m.len() == 1) {
                result(
                    json!({"recognized":true,"title":"Bridge Test Song","artist":"Bridge Test Artist"}),
                )
            } else {
                json!({"t":"QueryError","id":id,"code":"invalid"})
            }
        }
        Some("mac.network_check") => result(
            json!({"connected":true,"dns":true,"ping":"limited: ICMP unavailable","interfaces":[{"name":"en0","received_bytes":123,"sent_bytes":456}]}),
        ),
        Some("mac.clipboard_read") => result(json!({"text":lock(state)?.clipboard.clone()})),
        _ => return Ok(None),
    };
    Ok(Some(reply))
}

fn iwork(format: &str, text: &str) -> String {
    if format == "text" {
        return STANDARD.encode(text);
    }
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    let stream = format!("BT /F1 12 Tf 36 100 Td ({escaped}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = vec![];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = write!(out, "{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for offset in &offsets {
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }
    let _ = write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    );
    STANDARD.encode(out)
}

impl Session<'_> {
    pub(super) fn mac_prepare(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let permission = string(&lock(state)?.data, "notification_permission");
        if frame["action"]["op"] == "mac.notify"
            && !permission.is_empty()
            && permission != "authorized"
        {
            return self.reject(frame, "forbidden");
        }
        let ticket = super::id("ticket")?;
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
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":null,"route":"mac_readback","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    pub(super) fn mac_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            return self.reject(frame, "expired");
        }
        let action = &ticket.prepare["action"];
        let args = &action["args"];
        let op = string(action, "op");
        let mut s = lock(state)?;
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let readback = match op.as_str() {
            "mac.iwork_export" => {
                let document = STANDARD
                    .decode(string(args, "document_blob"))
                    .map_err(|_| invalid("document"))?;
                json!({"app":args["app"],"format":args["format"],"data_base64":iwork(&string(args,"format"), &String::from_utf8_lossy(&document))})
            }
            "mac.notify" => json!({"delivered":true}),
            "mac.open_settings" => json!({"pane":args["pane"],"frontmost":true}),
            "mac.set_wallpaper" => json!({"matches":true}),
            "mac.lock_screen" | "mac.sleep_display" | "mac.restart" | "mac.shutdown" => {
                json!({"dispatched":true})
            }
            "mac.clipboard_write" => {
                let Some(text) = args["text"].as_str() else {
                    drop(s);
                    return self.reject(frame, "invalid");
                };
                text.clone_into(&mut s.clipboard);
                json!(true)
            }
            "mac.set_volume" => args["volume"].clone(),
            "mac.set_mute" => args["muted"].clone(),
            "mac.set_appearance" => args["appearance"].clone(),
            "mac.keep_awake" => args["on"].clone(),
            _ => json!({"bundle_id":args["bundle_id"],"running":op != "mac.quit_app"}),
        };
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"observed":true,"fingerprint":ticket.prepare["request_hash"],"readback":readback}});
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}
