use crate::{
    mcp_answer::{self, DEFAULT_ITEMS, Window, page, paged, short},
    mcp_check::{Problem, mac_zone},
    mcp_facade::{Call, Step, compact},
};
use ab_catalog::AgentOperation;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const OVERDUE_TOP: usize = 3;
const NOTE_PREVIEW: usize = 100;
const UPCOMING_TOP: usize = 5;
const UPCOMING_DAYS: i64 = 7;
const VIEWS: &[&str] = &["due_today", "overdue", "upcoming"];

pub(crate) fn read_window(input: &Map<String, Value>, arguments: &Map<String, Value>) -> Window {
    let mut scope = input.clone();
    if !scope.contains_key("limit")
        && let Some(limit) = arguments.get("limit").filter(|l| l.is_u64())
    {
        scope.insert("limit".into(), limit.clone());
    }
    mcp_answer::window(&scope, DEFAULT_ITEMS)
}

fn zone() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::get(&mac_zone()).unwrap_or(jiff::tz::TimeZone::UTC)
}

fn today() -> jiff::civil::Date {
    jiff::Zoned::now().with_time_zone(zone()).date()
}

fn due_day(data: &Value) -> Option<jiff::civil::Date> {
    let due = data["due"].as_str()?;
    if due.len() == 10 {
        return due.parse().ok();
    }
    due.parse::<jiff::Timestamp>()
        .ok()
        .map(|at| at.to_zoned(zone()).date())
}

impl Call<'_> {
    pub(crate) fn check_view(operation: &AgentOperation, input: &Map<String, Value>) -> Step<()> {
        let Some(view) = input.get("view") else {
            return Ok(());
        };
        let smart_today = input
            .get("arguments")
            .is_some_and(|arguments| arguments["smart"] == "today");
        if operation.name != "reminders.items"
            || !smart_today
            || !view.as_str().is_some_and(|v| VIEWS.contains(&v))
        {
            return Err(Problem::new("invalid", "view only splits reminders.items with smart today")
                .field("view")
                .expected(json!({"view": VIEWS, "with": {"operation": "reminders.items", "arguments": {"smart": "today"}}})));
        }
        Ok(())
    }

    pub(crate) async fn agent_view(
        &self,
        operation: &AgentOperation,
        arguments: &Map<String, Value>,
        raw: &Value,
        shaped: Value,
        input: &Map<String, Value>,
    ) -> Step<Value> {
        let window = read_window(input, arguments);
        let shape = if input.contains_key("fields") {
            ""
        } else {
            operation.name.as_str()
        };
        match shape {
            "notes.list" => Ok(notes_view(raw, window)),
            "reminders.items" => {
                let names = self.list_names().await;
                if arguments.get("smart").and_then(Value::as_str) == Some("today") {
                    self.today_view(arguments, raw, &names, window, input).await
                } else {
                    let items = reminder_cards(raw, &names);
                    let (items, page) = page(
                        items,
                        window,
                        "Pass list for one list, smart for a smart list, or tag.",
                    );
                    Ok(
                        json!({"reminders": items, "page": page, "next": "Change one with apple_change, passing its id and version as expected_version."}),
                    )
                }
            }
            _ => {
                let mut value = shaped;
                paged(
                    &mut value,
                    window,
                    "Pass narrower arguments; apple_describe lists them.",
                    mcp_answer::upstream_cursor(raw).as_ref(),
                );
                Ok(value)
            }
        }
    }

    async fn list_names(&self) -> HashMap<String, String> {
        self.quiet("reminders.lists", json!({"time_zone": mac_zone()}))
            .await
            .map(|lists| {
                lists["lists"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        Some((
                            entry["list"]["id"].as_str()?.to_owned(),
                            entry["list"]["data"]["title"].as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn today_view(
        &self,
        arguments: &Map<String, Value>,
        raw: &Value,
        names: &HashMap<String, String>,
        window: Window,
        input: &Map<String, Value>,
    ) -> Step<Value> {
        let today = today();
        let mut due_today = Vec::new();
        let mut overdue = Vec::new();
        for (card, day) in reminder_cards(raw, names)
            .into_iter()
            .zip(reminder_days(raw))
        {
            match day {
                Some(day) if day < today => overdue.push((day, card)),
                _ => due_today.push(card),
            }
        }
        overdue.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let overdue: Vec<Value> = overdue.into_iter().map(|(_, card)| card).collect();
        let mut scheduled = arguments.clone();
        scheduled.insert("smart".into(), json!("scheduled"));
        let found = self
            .invoke(
                self.operation("reminders.items")?,
                &Value::Object(scheduled),
            )
            .await?;
        let last = today
            .checked_add(jiff::Span::new().days(UPCOMING_DAYS))
            .unwrap_or(today);
        let mut upcoming: Vec<(jiff::civil::Date, Value)> = reminder_cards(&found, names)
            .into_iter()
            .zip(reminder_days(&found))
            .filter_map(|(card, day)| day.filter(|d| *d > today && *d <= last).map(|d| (d, card)))
            .collect();
        upcoming.sort_by_key(|entry| entry.0);
        let upcoming: Vec<Value> = upcoming.into_iter().map(|(_, card)| card).collect();
        let call = |view: &str| json!({"tool": "apple_read", "arguments": {"operation": "reminders.items", "arguments": {"smart": "today"}, "view": view}});
        if let Some(view) = input.get("view").and_then(Value::as_str) {
            let chosen = match view {
                "overdue" => overdue,
                "upcoming" => upcoming,
                _ => due_today,
            };
            let (items, page) = page(chosen, window, "Pass list for one list, or tag.");
            return Ok(
                json!({"date": today.to_string(), "view": view, "reminders": items, "page": page}),
            );
        }
        let overdue_count = overdue.len();
        let upcoming_count = upcoming.len();
        let (due_today, due_page) = page(
            due_today,
            window,
            "Pass view overdue or upcoming for those lists.",
        );
        let mut value = json!({
            "date": today.to_string(),
            "due_today": {"reminders": due_today, "page": due_page},
            "overdue": {"count": overdue_count, "top": overdue.into_iter().take(OVERDUE_TOP).collect::<Vec<_>>()},
            "upcoming": {"days": UPCOMING_DAYS, "count": upcoming_count, "top": upcoming.into_iter().take(UPCOMING_TOP).collect::<Vec<_>>()},
            "next": "Complete one with apple_change reminders.complete, passing its id and version as expected_version.",
        });
        if overdue_count > OVERDUE_TOP {
            value["overdue"]["show_all"] = call("overdue");
        }
        if upcoming_count > UPCOMING_TOP {
            value["upcoming"]["show_all"] = call("upcoming");
        }
        Ok(value)
    }
}

fn reminder_records(raw: &Value) -> Vec<(&Value, &Value)> {
    raw["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| {
            section["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |item| (section, item))
        })
        .collect()
}

fn reminder_days(raw: &Value) -> Vec<Option<jiff::civil::Date>> {
    reminder_records(raw)
        .into_iter()
        .map(|(_, item)| due_day(&item["reminder"]["data"]))
        .collect()
}

fn reminder_cards(raw: &Value, names: &HashMap<String, String>) -> Vec<Value> {
    reminder_records(raw)
        .into_iter()
        .map(|(section, item)| {
            let record = compact(&item["reminder"], false, None);
            let data = &record["data"];
            let list = data["list_id"].as_str().unwrap_or_default();
            let mut card = json!({
                "id": record["id"],
                "version": record["version"],
                "title": data["title"],
                "due": data["due"],
                "due_local": data["due_local"],
                "list": names.get(list).map_or_else(|| json!(list), |name| json!(name)),
                "section": section["section"]["title"].as_str().filter(|t| !t.is_empty()),
                "notes": data["notes"].as_str().filter(|n| !n.is_empty()).map(short),
            });
            if data["priority"].as_u64().is_some_and(|p| p > 0) {
                card["priority"] = data["priority"].clone();
            }
            for flag in ["flagged", "completed"] {
                if data[flag] == true {
                    card[flag] = json!(true);
                }
            }
            if let Some(count) = item["subtasks"].as_array().map(Vec::len).filter(|n| *n > 0) {
                card["subtasks"] = json!(count);
            }
            if let Some(fields) = card.as_object_mut() {
                fields.retain(|_, v| !v.is_null());
            }
            card
        })
        .collect()
}

fn notes_view(raw: &Value, window: Window) -> Value {
    let folders: Vec<Value> = raw["folders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["recently_deleted"] != true)
        .map(|f| json!({"id": f["folder"]["id"], "title": f["folder"]["data"]["title"], "count": f["count"]}))
        .collect();
    let names: HashMap<&str, &str> = raw["folders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some((
                f["folder"]["id"].as_str()?,
                f["folder"]["data"]["title"].as_str()?,
            ))
        })
        .collect();
    let notes: Vec<Value> = raw["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| {
            let names = &names;
            section["notes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |item| {
                    let record = &item["note"];
                    let data = &record["data"];
                    let folder = data["folder_id"].as_str().unwrap_or_default();
                    let mut card = json!({
                        "id": record["id"],
                        "version": record["version"],
                        "title": data["title"],
                        "folder": names.get(folder).copied().unwrap_or(folder),
                        "modified_at": data["modified_at"],
                        "preview": item["preview"].as_str().filter(|p| !p.is_empty()).map(|p| mcp_answer::clip(p.trim(), NOTE_PREVIEW).0),
                        "section": section["title"],
                    });
                    for flag in ["pinned", "locked"] {
                        if data[flag] == true || item[flag] == true {
                            card[flag] = json!(true);
                        }
                    }
                    if let Some(fields) = card.as_object_mut() {
                        fields.retain(|_, v| !v.is_null());
                    }
                    card
                })
        })
        .collect();
    let (notes, page) = page(
        notes,
        window,
        "Pass folder (a fld_ ID from folders) for one folder, or find words with apple_search sources notes.",
    );
    let (folders, folder_page) = mcp_answer::page(
        folders,
        Window {
            offset: 0,
            limit: mcp_answer::MAX_ITEMS,
        },
        "Folders are listed in full up to 100.",
    );
    json!({
        "notes": notes,
        "page": page,
        "folders": folders,
        "folders_page": folder_page,
        "next": "Read one note's full text with apple_read records.get and its id.",
    })
}

pub(crate) fn search_card(record: &Value) -> Value {
    let record = compact(record, false, None);
    let data = &record["data"];
    let first = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| data[*key].as_str().filter(|v| !v.trim().is_empty()))
            .map(str::to_owned)
    };
    let mut card = json!({
        "id": record["id"],
        "version": record["version"],
        "title": first(&["title", "subject", "name", "display_name", "file_name"]),
        "from": data["from"],
        "when": first(&["modified_at", "received_at", "sent_at", "start", "due", "created_at", "date"]),
        "preview": first(&["snippet", "preview", "plain_text", "text", "transcript", "notes", "body", "location", "url", "path"]).map(|text| short(&text)),
    });
    if let Some(fields) = card.as_object_mut() {
        fields.retain(|_, v| !v.is_null());
    }
    card
}

pub(crate) fn contact_card(record: &Value) -> Value {
    let data = &record["data"];
    let values = |key: &str| -> Option<Vec<Value>> {
        let found: Vec<Value> = data[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let value = entry["value"].as_str().or_else(|| entry.as_str())?;
                Some(match entry["label"].as_str().filter(|l| !l.is_empty()) {
                    Some(label) => json!({"label": label, "value": value}),
                    None => json!({"value": value}),
                })
            })
            .collect();
        (!found.is_empty()).then_some(found)
    };
    let name = ["given_name", "middle_name", "family_name"]
        .iter()
        .filter_map(|key| data[*key].as_str().filter(|v| !v.trim().is_empty()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut card = json!({
        "id": record["id"],
        "version": record["version"],
        "name": (!name.is_empty()).then_some(name),
        "nickname": data["nickname"],
        "organization": data["organization"],
        "job_title": data["job_title"],
        "phones": values("phones"),
        "emails": values("emails"),
        "birthday": data["birthday"],
    });
    if let Some(fields) = card.as_object_mut() {
        fields.retain(|_, v| !v.is_null() && v.as_str() != Some(""));
    }
    card
}

pub(crate) fn chat_card(summary: &Value) -> Value {
    let mut card = summary.clone();
    if let Some(fields) = card.as_object_mut() {
        if let Some(everyone) = fields
            .remove("participants")
            .as_ref()
            .and_then(Value::as_array)
            && summary["group"] == true
        {
            fields
                .entry("participant_count")
                .or_insert_with(|| json!(everyone.len()));
        }
        if let Some(last) = fields
            .get_mut("last_message")
            .and_then(Value::as_object_mut)
        {
            let preview = last.remove("preview").unwrap_or_default();
            last.remove("id");
            last.remove("sender_handle_id");
            if last.get("service") == summary.get("service") {
                last.remove("service");
            }
            if let Some(text) = preview["text"].as_str() {
                last.insert("text".into(), json!(text));
            }
            if let Some(kind) = preview["kind"].as_str().filter(|k| *k != "text") {
                last.insert("kind".into(), json!(kind));
            }
            if let Some(reaction) = preview.get("reaction").filter(|r| !r.is_null()) {
                last.insert("reaction".into(), reaction.clone());
            }
        }
    }
    card
}
