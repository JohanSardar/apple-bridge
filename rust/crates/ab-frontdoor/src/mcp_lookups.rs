use crate::{
    mcp_check::{Problem, from_error, mac_zone},
    mcp_facade::{Call, Step, local_time_of, text},
};
use ab_operations::composite_reads::{FreeSlotsQuery, SharedChat, SharedKind, SharedQuery};
use jiff::{SignedDuration, Timestamp, civil::Time, tz::TimeZone};
use serde_json::{Map, Value, json};

pub(crate) const FREE_SLOTS: &str = "calendar.free_slots";
pub(crate) const SHARED: &str = "messages.shared";
pub(crate) const SHARED_KINDS: &[&str] = &[
    "any", "image", "video", "audio", "pdf", "document", "file", "link",
];

impl Call<'_> {
    pub(crate) fn free_slots(&self, arguments: &Map<String, Value>) -> Step<Value> {
        let zone_name = mac_zone();
        let start =
            text(arguments, "start").map_or(Ok(Timestamp::now()), |raw| instant("start", raw))?;
        let end = text(arguments, "end").map_or_else(
            || {
                start
                    .checked_add(SignedDuration::from_hours(24 * 7))
                    .map_err(|_| Problem::new("invalid", "end is out of range").field("end"))
            },
            |raw| instant("end", raw),
        )?;
        let query = FreeSlotsQuery {
            start,
            end,
            zone: TimeZone::get(&zone_name).unwrap_or(TimeZone::UTC),
            day_start: clock("day_start", text(arguments, "day_start").unwrap_or("09:00"))?,
            day_end: clock("day_end", text(arguments, "day_end").unwrap_or("17:00"))?,
            weekends: arguments.get("weekends").and_then(Value::as_bool) == Some(true),
            minutes: arguments
                .get("minutes")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(30),
            limit: count(arguments, "limit", 10),
            calendar: text(arguments, "calendar").map(str::to_owned),
        };
        let slots = self
            .frontdoor
            .service
            .calendar_free_slots(self.principal, &query)
            .map_err(|error| from_error(&error, self.registry.find(FREE_SLOTS), self.principal))?;
        let local = |stamp: &str| {
            stamp.parse::<Timestamp>().ok().map(|t| {
                t.to_zoned(query.zone.clone())
                    .strftime("%a %Y-%m-%d %H:%M")
                    .to_string()
            })
        };
        let slots: Vec<Value> = slots
            .into_iter()
            .map(|slot| {
                json!({
                    "start": slot.start,
                    "end": slot.end,
                    "start_local": local(&slot.start),
                    "end_local": local(&slot.end),
                    "minutes": slot.minutes,
                })
            })
            .collect();
        let mut value = json!({
            "slots": slots,
            "minutes": query.minutes,
            "time_zone": zone_name,
            "searched": {"start": start.to_string(), "end": end.to_string(), "day_start": &query.day_start.to_string()[..5], "day_end": &query.day_end.to_string()[..5], "weekends": query.weekends},
            "next": "To book a slot, call calendar_add with a start inside it.",
        });
        if let Some(calendar) = query.calendar {
            value["calendar"] = json!(calendar);
        }
        Ok(value)
    }

    pub(crate) async fn shared(&self, arguments: &Map<String, Value>) -> Step<Value> {
        let chat = match (text(arguments, "chat_id"), text(arguments, "with")) {
            (Some(id), None) => SharedChat::Id(id.to_owned()),
            (None, Some(who)) => SharedChat::DirectHandles(self.person(who, "with").await?.handles),
            _ => {
                return Err(
                    Problem::new("invalid", "give exactly one of with or chat_id").field("with"),
                );
            }
        };
        let kind: SharedKind = serde_json::from_value(json!(
            text(arguments, "kind").unwrap_or("any")
        ))
        .map_err(|_| {
            Problem::new("invalid", "unknown kind")
                .field("kind")
                .expected(json!(SHARED_KINDS))
        })?;
        let query = SharedQuery {
            chat,
            kind,
            limit: count(arguments, "limit", 20),
            scan: count(arguments, "scan", 600),
        };
        let result = self
            .frontdoor
            .service
            .messages_shared(self.principal, &query)
            .map_err(|error| from_error(&error, self.registry.find(SHARED), self.principal))?;
        let summaries = result
            .items
            .iter()
            .map(|item| {
                json!({
                    "sender_handle_id": item["from_handle_id"],
                    "from_me": item["from_me"],
                })
            })
            .collect();
        let summaries = self.with_names(summaries).await;
        let items: Vec<Value> = result
            .items
            .into_iter()
            .zip(summaries)
            .map(|(mut item, summary)| {
                if let Some(local) = local_time_of(item["sent_at"].as_str()) {
                    item["sent_at_local"] = json!(local);
                }
                item["from"] = summary["from"].clone();
                if let Some(fields) = item.as_object_mut() {
                    fields.remove("from_handle_id");
                    fields.remove("from_me");
                }
                item
            })
            .collect();
        Ok(json!({
            "chat_id": result.chat_id,
            "kind": kind,
            "items": items,
            "examined_messages": result.examined_messages,
            "complete": result.complete,
            "next": if result.complete {
                "Get a file with apple_read bridge.read_blob, using its attachment_id as record_id and its digest."
            } else {
                "Raise scan or limit for older items. Get a file with apple_read bridge.read_blob, using its attachment_id as record_id and its digest."
            },
        }))
    }
}

fn count(arguments: &Map<String, Value>, key: &str, default: usize) -> usize {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default)
}

fn instant(field: &str, raw: &str) -> Step<Timestamp> {
    crate::mcp_compose::moment(field, raw, false)?
        .parse()
        .map_err(|_| Problem::new("invalid", format!("{field} is not a time")).field(field))
}

fn clock(field: &str, raw: &str) -> Step<Time> {
    raw.parse::<Time>().map_err(|_| {
        Problem::new("invalid", format!("{field} is not a time of day"))
            .field(field)
            .expected(json!("HH:MM, such as 09:00"))
    })
}
