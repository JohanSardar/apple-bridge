use super::{Observation, Reader, Result, Row, dataless, reaction, reference, target};
use crate::{body_digest, rich, summary, typedstream};
use serde_json::{Value, json};
use std::{fs, io::Read};

fn time(raw: f64) -> Option<String> {
    if !raw.is_finite() {
        return None;
    }
    let seconds = raw + 978_307_200.0;
    #[allow(clippy::cast_possible_truncation)]
    let millis = (seconds * 1000.0).round() as i64;
    jiff::Timestamp::from_millisecond(millis)
        .ok()
        .map(|t| format!("{t:.3}"))
}
fn timestamp(row: &Row, key: &str) -> Option<String> {
    let n = row.text(key).parse::<f64>().ok()?;
    (n > 0.0)
        .then(|| time(if n > 1e12 { n / 1e9 } else { n }))
        .flatten()
}
fn optional(data: &mut Value, field: &str, value: Option<impl Into<Value>>) {
    if let Some(v) = value {
        data[field] = v.into();
    }
}
impl Reader {
    pub(super) fn message_observations(&self, row: &Row) -> Vec<Observation> {
        let guid = row.text("guid");
        if guid.is_empty() {
            return Vec::new();
        }
        let sender = row
            .values
            .get("capture_sender_key")
            .map(|s| reference("handle", s));
        if let Some((part, target, removed, kind)) = reaction(row) {
            let mut data = json!({"message_id":reference("message",target),"part_index":part,"kind":kind,"removed":removed,"is_from_me":row.flag("is_from_me")});
            optional(&mut data, "sent_at", timestamp(row, "date"));
            optional(
                &mut data,
                "emoji",
                row.values.get("associated_message_emoji").cloned(),
            );
            optional(&mut data, "sender_handle_id", sender);
            return vec![Observation::new("reaction", guid, data)];
        }
        let mut data = json!({"is_from_me":row.flag("is_from_me"),"kind":if row.int("item_type")==0{"message"}else{"group_event"},"error_code":row.int("error"),"edit_history":[],"retracted_parts":[],"formatting":[]});
        if !row.text("capture_chat_guid").is_empty() {
            data["chat_id"] = reference("chat", row.text("capture_chat_guid"));
        }
        optional(
            &mut data,
            "deleted_at",
            timestamp(row, "capture_deleted_date"),
        );
        optional(&mut data, "sender_handle_id", sender.clone());
        if !row.text("service").is_empty() {
            data["service"] = json!(row.text("service"));
        }
        for (native, field) in [
            ("date", "sent_at"),
            ("date_delivered", "delivered_at"),
            ("date_read", "read_at"),
            ("date_edited", "edited_at"),
            ("date_retracted", "retracted_at"),
        ] {
            optional(&mut data, field, timestamp(row, native));
        }
        for (native, field) in [
            ("reply_to_guid", "reply_to_id"),
            ("thread_originator_guid", "thread_originator_id"),
        ] {
            if !row.text(native).is_empty() {
                data[field] = reference("message", target(row.text(native)));
            }
        }
        let mut parts = body_parts(row, &mut data);
        optional(&mut data, "subject", row.values.get("subject").cloned());
        data["delivery"] = json!({"quietly":row.flag("was_delivered_quietly"),"notified_anyway":row.flag("did_notify_recipient")});
        for key in ["is_spam", "has_unseen_mention", "is_sos"] {
            data[key] = json!(row.flag(key));
        }
        data["via_satellite"] =
            json!(row.flag("is_stewie") || row.flag("sent_or_received_off_grid"));
        let mut audio = json!({"is_audio_message":row.flag("is_audio_message"),"expirable":row.flag("is_expirable"),"kept":null});
        optional(&mut audio, "played_at", timestamp(row, "date_played"));
        data["audio"] = audio;
        if row.int("schedule_type") != 0 {
            let mut schedule = json!({"state":format!("native_{}",row.text("schedule_state"))});
            optional(&mut schedule, "scheduled_for", timestamp(row, "date"));
            data["schedule"] = schedule;
        }
        if data.get("deleted_at").is_some() {
            data["recoverable_until"] = Value::Null;
        }
        self.rich_parts(row, &mut data, &mut parts);
        data["parts"] = json!(parts);
        if !row.text("expressive_send_style_id").is_empty() {
            data["effect"] = json!(row.text("expressive_send_style_id"));
        }
        let edit_count = message_summary(row, &mut data);
        if row.text("service") == "iMessage"
            && row.flag("is_from_me")
            && row.int("item_type") == 0
            && let Some(raw) = row.text("date").parse::<f64>().ok().filter(|n| *n > 0.0)
        {
            let seconds = if raw > 1e12 { raw / 1e9 } else { raw };
            if edit_count < 5 {
                optional(&mut data, "edit_until", time(seconds + 900.0));
            }
            optional(&mut data, "unsend_until", time(seconds + 120.0));
        }
        if row.int("item_type") != 0 {
            let mut event = json!({"item_type":row.int("item_type"),"action_type":row.int("group_action_type"),"title":row.text("group_title"),"kind":rich::group_event_kind(row.int("item_type"),row.int("group_action_type"),row.int("other_handle")>0,row.int("handle_id")==row.int("other_handle"),row.text("group_title"))});
            if row.int("item_type") == 1 && !row.text("capture_member_key").is_empty() {
                event["target_handle_id"] = reference("handle", row.text("capture_member_key"));
            }
            optional(&mut event, "actor_handle_id", sender);
            data["group_event"] = event;
        }
        vec![Observation::new("message", guid, data)]
    }
    fn rich_parts(&self, row: &Row, data: &mut Value, parts: &mut [Value]) {
        if !row.text("balloon_bundle_id").is_empty() || parts.iter().any(|p| p["kind"] == "genmoji")
        {
            let attachments = &row.attachments;
            let images = attachments
                .iter()
                .filter(|r| {
                    self.mime(r).starts_with("image/") || r.text("uti").starts_with("dyn.age81")
                })
                .map(|r| r.text("guid").to_string())
                .collect::<Vec<_>>();
            if let Some(mut balloon) = rich::balloon(
                row.text("balloon_bundle_id"),
                row.blobs.get("payload_data").map(Vec::as_slice),
                &images,
            ) {
                for key in ["link", "app"] {
                    for field in ["image_attachment_id", "icon_attachment_id"] {
                        if let Some(s) = balloon[key][field].as_str().map(ToString::to_string) {
                            balloon[key][field] = reference("attachment", &s);
                        }
                    }
                }
                data["balloon"] = balloon;
            }
            for part in parts.iter_mut() {
                if part["kind"] == "genmoji" {
                    let glyphs = attachments
                        .iter()
                        .filter(|r| !r.text("emoji_image_content_identifier").is_empty())
                        .collect::<Vec<_>>();
                    let target = part
                        .pointer("/attachment_id/$ref/native_key")
                        .and_then(Value::as_str);
                    if let Some(r) = glyphs
                        .iter()
                        .find(|r| Some(r.text("guid")) == target)
                        .copied()
                        .or_else(|| (glyphs.len() == 1).then(|| glyphs[0]))
                    {
                        part["attachment_id"] = reference("attachment", r.text("guid"));
                        part["description"] = json!(r.text("emoji_image_short_description"));
                    }
                }
            }
        }
    }
    pub(super) fn small_observations(&self) -> Result<Vec<Observation>> {
        let mut items = Vec::new();
        if self.has("handle", &["id", "service"]) {
            for row in self.query("SELECT * FROM handle", &[])? {
                items.push(Observation::new(
                    "handle",
                    &format!("{}:{}", row.text("service"), row.text("id")),
                    json!({"address":row.text("id"),"service":row.text("service")}),
                ));
            }
        }
        if self.has("chat", &["guid"]) {
            for row in self.query(
                &format!(
                    "SELECT c.ROWID AS row_position,c.*,{} AS capture_deleted_date FROM chat c",
                    self.deletion(true)
                ),
                &[],
            )? {
                let participants = if self.has("handle", &["id", "service"])
                    && self.has("chat_handle_join", &["chat_id", "handle_id"])
                {
                    self.query("SELECT h.id,h.service FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id WHERE j.chat_id=? ORDER BY h.service,h.id",&[row.text("row_position").into()])?.iter().map(|r|reference("handle",&format!("{}:{}",r.text("service"),r.text("id")))).collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let mut data = json!({"service":row.text("service_name"),"style":if row.text("guid").contains(";+;"){"group"}else{"direct"},"display_name":row.text("display_name"),"participants":participants,"is_archived":row.flag("is_archived"),"capabilities":[],"filter":if row.flag("is_filtered"){"unknown"}else{"known"},"is_spam":row.int("is_filtered")==2});
                optional(
                    &mut data,
                    "last_read_at",
                    timestamp(&row, "last_read_message_timestamp"),
                );
                optional(
                    &mut data,
                    "deleted_at",
                    timestamp(&row, "capture_deleted_date"),
                );
                if data.get("deleted_at").is_some() {
                    data["recoverable_until"] = Value::Null;
                }
                if let Value::Object(props) =
                    rich::chat_properties(row.blobs.get("properties").map(Vec::as_slice))
                {
                    for (k, v) in props {
                        data[&k] = v;
                    }
                }
                if let Some(s) = data["group_photo_attachment_id"]
                    .as_str()
                    .map(ToString::to_string)
                {
                    data["group_photo_attachment_id"] = reference("attachment", &s);
                }
                if let Some(index) = self
                    .pins
                    .as_ref()
                    .and_then(|p| p.iter().position(|s| s == row.text("chat_identifier")))
                    .filter(|_| !row.text("chat_identifier").is_empty())
                {
                    data["pinned_index"] = json!(index);
                }
                items.push(Observation::new("chat", row.text("guid"), data));
            }
        }
        if self.has("attachment", &["guid"]) {
            let message = if self.has("message_attachment_join", &["message_id", "attachment_id"]) {
                "(SELECT m.guid FROM message_attachment_join j JOIN message m ON m.ROWID=j.message_id WHERE j.attachment_id=a.ROWID ORDER BY m.ROWID LIMIT 1)"
            } else {
                "NULL"
            };
            for row in self.query(&format!("SELECT a.ROWID AS row_position,a.*,{message} AS message_guid FROM attachment a ORDER BY a.ROWID DESC"),&[])? {
                let path=self.attachment_path(row.text("filename"));
                let meta=path.as_ref().and_then(|p|fs::metadata(p).ok());
                let state=if meta.as_ref().is_some_and(dataless){"offloaded"}else if meta.is_some(){"present"}else if row.int("transfer_state")==0{"pending"}else if row.text("filename").is_empty(){"missing"}else{"offloaded"};
                let mut blob=json!({"state":state});if meta.is_some(){blob["handle"]=json!(row.text("guid"));}
                let mut data=json!({"filename":std::path::Path::new(row.text("filename")).file_name().unwrap_or_default().to_string_lossy(),"mime_type":self.mime(&row),"uti":row.text("uti"),"size":row.int("total_bytes"),"blob":blob,"is_sticker":row.flag("is_sticker"),"hidden":row.flag("hide_attachment"),"genmoji":!row.text("emoji_image_content_identifier").is_empty()});
                if !row.text("message_guid").is_empty(){data["message_id"]=reference("message",row.text("message_guid"));}
                if !row.text("emoji_image_short_description").is_empty(){data["description"]=json!(row.text("emoji_image_short_description"));}
                if let Some(bytes)=row.blobs.get("sticker_user_info").filter(|_|self.has("message",&["associated_message_guid"]))
                    && let Some(parent)=self.query("SELECT associated_message_guid FROM message WHERE guid=?",&[row.text("message_guid").into()])?.first()
                        && let Some(mut sticker)=rich::sticker(bytes,parent.text("associated_message_guid")) {if let Some(s)=sticker["target_message_id"].as_str().map(ToString::to_string){sticker["target_message_id"]=reference("message",&s);}data["sticker"]=sticker;}
                items.push(Observation::new("attachment",row.text("guid"),data));
            }
        }
        Ok(items)
    }
    fn mime(&self, row: &Row) -> String {
        if row.text("uti").starts_with("dyn.age81")
            && let Some(path) = self.attachment_path(row.text("filename"))
            && fs::metadata(&path).is_ok_and(|m| !dataless(&m))
            && let Ok(mut file) = fs::File::open(path)
            && file.metadata().is_ok_and(|m| !dataless(&m))
        {
            let mut bytes = [0; 12];
            let n = file.read(&mut bytes).unwrap_or(0);
            let b = &bytes[..n];
            for (prefix, mime) in [
                (&[137, 80, 78, 71, 13, 10, 26, 10][..], "image/png"),
                (&[255, 216, 255][..], "image/jpeg"),
                (b"GIF8".as_slice(), "image/gif"),
            ] {
                if b.starts_with(prefix) {
                    return mime.into();
                }
            }
        }
        row.text("mime_type").into()
    }
}

fn body_parts(row: &Row, data: &mut Value) -> Vec<Value> {
    let mut parts = Vec::new();
    if let Some(bytes) = row.blobs.get("attributedBody") {
        let body = typedstream::decode(bytes);
        data["decode_status"] = json!(body.decode_status);
        optional(data, "raw_body_digest", body.raw_body_digest);
        optional(data, "text", body.text);
        data["formatting"]=Value::Array(body.formatting.into_iter().map(|r| {let mut v=json!({"utf16_location":r.utf16_location,"utf16_length":r.utf16_length,"styles":r.styles}); optional(&mut v,"effect",r.effect); v}).collect());
        for part in body.parts {
            let mut v = json!({"index":part.index,"kind":part.kind,"utf16_location":part.utf16_location,"utf16_length":part.utf16_length});
            optional(&mut v, "text", part.text);
            optional(&mut v, "mention", part.mention);
            optional(
                &mut v,
                "attachment_id",
                part.attachment_guid.map(|s| reference("attachment", &s)),
            );
            parts.push(v);
        }
    } else if let Some(text) = row.values.get("text") {
        data["text"] = json!(text);
        data["decode_status"] = json!("decoded");
        parts.push(json!({"index":0,"kind":"text","text":text}));
    } else {
        data["decode_status"] = json!(
            if row.int("date_retracted") > 0 || row.int("item_type") != 0 {
                "decoded"
            } else {
                "undecoded"
            }
        );
        data["raw_body_digest"] = json!(body_digest(
            row.blobs.get("text").map_or(&[], Vec::as_slice)
        ));
    }

    parts
}

fn message_summary(row: &Row, data: &mut Value) -> usize {
    let mut edit_count = 0;
    if let Some(bytes) = row.blobs.get("message_summary_info") {
        let summary = summary::decode(bytes);
        edit_count = summary.edit_history.len();
        data["edit_history"] = Value::Array(
            summary
                .edit_history
                .into_iter()
                .map(|e| {
                    let mut v =
                        json!({"part_index":e.part_index,"decode_status":e.body.decode_status});
                    optional(&mut v, "text", e.body.text);
                    optional(&mut v, "timestamp", e.timestamp.and_then(time));
                    optional(&mut v, "raw_body_digest", e.body.raw_body_digest);
                    v
                })
                .collect(),
        );
        data["retracted_parts"] = json!(summary.retracted_parts);
        if summary.decode_status != "decoded" {
            data["summary_decode_status"] = json!(summary.decode_status);
            optional(data, "summary_raw_digest", summary.raw_body_digest);
            if data["decode_status"] == "decoded" {
                data["decode_status"] = json!("partial");
            }
        }
    }

    edit_count
}
