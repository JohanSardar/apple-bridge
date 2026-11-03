#![allow(clippy::too_many_lines)]
use crate::{Dataset, EPOCH, add_date, date_only, handle_key, message_key, reference, ts};
use serde_json::{Map, Value, json};

pub fn reactions(d: &mut Dataset) {
    let kinds = [
        "love",
        "like",
        "dislike",
        "laugh",
        "emphasize",
        "question",
        "emoji",
    ];
    for (i, kind) in kinds.iter().enumerate() {
        for sender in 0..3_usize {
            let mut fields = json!({
                "message_id": reference("messages.message", &message_key(0, 50 + i)),
                "part_index": 0,
                "sender_handle_id": reference("messages.handle", &handle_key(sender)),
                "is_from_me": sender == 0,
                "kind": kind,
                "removed": false,
                "sent_at": ts(EPOCH - (60 - i64::try_from(i).unwrap_or_default()) * 60),
            });
            if *kind == "emoji" {
                fields["emoji"] = json!("🪁");
            }
            d.put(
                "messages.reaction",
                &format!("demo-reaction-{i:02}-{sender}"),
                fields,
            );
        }
    }
    d.put(
        "messages.reaction",
        "demo-reaction-removed",
        json!({
            "message_id": reference("messages.message", &message_key(0, 57)),
            "part_index": 0,
            "sender_handle_id": reference("messages.handle", &handle_key(4)),
            "kind": "love",
            "removed": true,
            "is_from_me": false,
            "sent_at": ts(EPOCH - 60),
        }),
    );
}

fn coverage(fields: &[&str]) -> Map<String, Value> {
    fields
        .iter()
        .map(|field| ((*field).to_owned(), json!({"status": "complete"})))
        .collect()
}

struct List {
    key: &'static str,
    title: &'static str,
    emblem: &'static str,
    group: bool,
    pinned: bool,
    parent: &'static str,
    sections: Vec<Value>,
}

fn section(id: &str, title: &str, order: i64) -> Value {
    json!({"kind": "store", "id": id, "title": title, "order": order})
}

fn assignment(id: &str, me: bool) -> Value {
    json!({"assignee": {"id": id, "is_me": me}, "originator": null, "assigned_at": ts(EPOCH)})
}

fn lists(d: &mut Dataset) {
    let all = [
        List {
            key: "demo-rem-home",
            title: "Home",
            emblem: "🏠",
            group: true,
            pinned: false,
            parent: "",
            sections: vec![],
        },
        List {
            key: "list",
            title: "Personal",
            emblem: "☀️",
            group: false,
            pinned: false,
            parent: "demo-rem-home",
            sections: vec![
                section("demo-rem-now", "This week", 0),
                section("demo-rem-later", "Later", 1),
            ],
        },
        List {
            key: "demo-rem-groceries",
            title: "Groceries",
            emblem: "🛒",
            group: false,
            pinned: false,
            parent: "demo-rem-home",
            sections: vec![
                section("demo-rem-produce", "Produce", 0),
                section("demo-rem-pantry", "Pantry", 1),
            ],
        },
        List {
            key: "demo-rem-projects",
            title: "Projects",
            emblem: "💻",
            group: false,
            pinned: false,
            parent: "",
            sections: vec![],
        },
        List {
            key: "demo-rem-travel",
            title: "Travel",
            emblem: "✈️",
            group: false,
            pinned: true,
            parent: "",
            sections: vec![],
        },
        List {
            key: "demo-rem-smart",
            title: "Flagged errands",
            emblem: "🚩",
            group: false,
            pinned: false,
            parent: "",
            sections: vec![],
        },
    ];
    for (index, list) in all.into_iter().enumerate() {
        let parent = if list.parent.is_empty() {
            Value::Null
        } else {
            reference("reminders.list", list.parent)
        };
        let mut fields = json!({
            "title": list.title,
            "writable": !list.group,
            "color": "#4B8CF5",
            "account": "demo-account",
            "store_identifier": list.key,
            "is_group": list.group,
            "parent_id": parent,
            "badge_emblem": list.emblem,
            "display_order": index,
            "pinned": list.pinned,
            "pinned_at": null,
            "sorting_style": "manual",
            "categorize_groceries": list.key == "demo-rem-groceries",
            "grocery_locale": "en_US",
            "sharing_status": 0,
            "shared_owner_name": null,
            "smart_list_type": null,
            "smart_filter": null,
            "sections": list.sections,
            "store_color": null,
            "store_status": "partial",
        });
        let mut cover = coverage(&[
            "store_identifier",
            "is_group",
            "parent_id",
            "badge_emblem",
            "display_order",
            "pinned",
            "pinned_at",
            "sorting_style",
            "categorize_groceries",
            "grocery_locale",
            "sharing_status",
            "shared_owner_name",
            "smart_list_type",
            "sections",
        ]);
        cover.insert("store_color".into(), json!({"status": "unsupported"}));
        cover.insert("smart_filter".into(), json!({"status": "unsupported"}));
        if list.pinned {
            fields["pinned_at"] = json!(ts(EPOCH - 24 * 3600));
        }
        if list.key == "demo-rem-smart" {
            fields["smart_list_type"] = json!("custom");
            fields["writable"] = json!(false);
        }
        fields["store_coverage"] = Value::Object(cover);
        d.put("reminders.list", list.key, fields);
    }
}

pub fn reminders(d: &mut Dataset) {
    lists(d);
    let rem = |key: &str| reference("reminders.reminder", key);
    let items: Vec<(&str, &str, &str, Value)> = vec![
        (
            "demo-rem-plan",
            "Plan a weekend trip",
            "demo-rem-travel",
            json!({"flagged": true, "due": date_only(add_date(EPOCH, 0, 0, 3)), "tags": ["travel"], "priority": 1, "assignment": assignment("demo-me", true)}),
        ),
        (
            "demo-rem-stay",
            "Book a place to stay",
            "demo-rem-travel",
            json!({"parent_id": rem("demo-rem-plan"), "tags": ["travel"]}),
        ),
        (
            "demo-rem-options",
            "Compare two quiet cabins",
            "demo-rem-travel",
            json!({"parent_id": rem("demo-rem-stay"), "tags": ["travel"]}),
        ),
        (
            "demo-rem-walk",
            "Take a long walk",
            "list",
            json!({"section_id": "demo-rem-now", "due": date_only(EPOCH), "tags": ["home"]}),
        ),
        (
            "demo-rem-plants",
            "Water the plants",
            "list",
            json!({"section_id": "demo-rem-later", "priority": 9, "tags": ["home"]}),
        ),
        (
            "demo-rem-market",
            "Pick up dinner ingredients",
            "demo-rem-groceries",
            json!({"flagged": true, "section_id": "demo-rem-produce", "priority": 5, "tags": ["errands"], "due": date_only(add_date(EPOCH, 0, 0, 1))}),
        ),
        (
            "demo-rem-tomatoes",
            "Tomatoes and basil",
            "demo-rem-groceries",
            json!({"section_id": "demo-rem-produce", "tags": ["errands"]}),
        ),
        (
            "demo-rem-pasta",
            "Pasta and olive oil",
            "demo-rem-groceries",
            json!({"section_id": "demo-rem-pantry"}),
        ),
        (
            "demo-rem-review",
            "Review the first draft",
            "demo-rem-projects",
            json!({"due": ts(EPOCH + 48 * 3600), "due_all_day": false, "early_reminders": [-900], "tags": ["work"], "assignment": assignment("demo-other", false)}),
        ),
        (
            "demo-rem-done",
            "Choose the next book",
            "list",
            json!({"completed": true, "completed_at": ts(EPOCH - 3600), "section_id": "demo-rem-now", "tags": ["home"]}),
        ),
    ];
    for (index, (key, title, list, extra)) in items.into_iter().enumerate() {
        let mut fields = json!({
            "title": title,
            "list_id": reference("reminders.list", list),
            "completed": false,
            "completed_at": null,
            "priority": 0,
            "due": null,
            "due_all_day": true,
            "notes": null,
            "url": null,
            "flagged": false,
            "parent_id": null,
            "section_id": null,
            "tags": [],
            "manual_order": index,
            "created_at": ts(EPOCH - 48 * 3600),
            "early_reminders": [],
            "assignment": null,
            "store_alarms": [],
            "store_recurrence": [],
            "attachment_ids": [],
            "store_identifier": key,
            "store_status": "complete",
        });
        for (name, value) in extra.as_object().into_iter().flatten() {
            fields[name] = value.clone();
        }
        fields["store_coverage"] = Value::Object(coverage(&[
            "store_identifier",
            "flagged",
            "parent_id",
            "section_id",
            "tags",
            "manual_order",
            "created_at",
            "early_reminders",
            "assignment",
            "store_alarms",
            "store_recurrence",
            "attachment_ids",
        ]));
        d.put("reminders.reminder", key, fields);
    }
}

#[allow(clippy::too_many_lines)]
pub fn calendars(d: &mut Dataset) {
    let calendars = [
        ("demo-cal-work", "Work", "#f08a24", "caldav", true),
        ("demo-cal-personal", "Personal", "#4b8cf5", "caldav", true),
        ("demo-cal-family", "Family", "#b066d1", "caldav", true),
        (
            "demo-cal-birthdays",
            "Birthdays",
            "#e4af26",
            "birthday",
            false,
        ),
        (
            "demo-cal-holidays",
            "Holidays",
            "#5fa16e",
            "subscribed",
            false,
        ),
    ];
    for (key, title, color, kind, writable) in calendars {
        d.put(
            "calendar.calendar",
            key,
            json!({"title": title, "account": "demo-account", "color": color, "type": kind, "writable": writable}),
        );
    }
    let midnight = EPOCH - 16 * 3600;
    for day in -14_i64..=42 {
        let date = midnight + day * 86400;
        for index in 0..3_usize {
            let title = ["Project check-in", "Lunch with Sam", "Evening walk"][index];
            let start = date + [14, 17, 22][index] * 3600;
            let location = ["Studio", "Corner café", "Riverside park"][index];
            let mut fields = json!({
                "calendar_id": reference("calendar.calendar", calendars[index].0),
                "title": title,
                "start": ts(start),
                "end": ts(start + 3600),
                "all_day": false,
                "time_zone": "America/New_York",
                "location": location,
                "notes": "",
                "url": null,
                "status": 1,
                "availability": 0,
                "is_exception": false,
                "attendees": [],
                "alarms": [],
                "recurrence": [],
                "store_status": "partial",
                "store_coverage": {
                    "travel_time": {"status": "unsupported", "reason": "positive native fixture pending"},
                    "structured_location": {"status": "unsupported", "reason": "native coordinate units unverified"},
                },
                "attachments": [],
            });
            if index == 0 {
                fields["recurrence"] = json!([{"frequency": "daily", "interval": 1, "count": 0}]);
                fields["series_id"] = json!("demo-daily-checkin");
                fields["occurrence_date"] = json!(ts(start));
                fields["notes"] = json!("Review the draft, then choose the next small step.");
                fields["alarms"] = json!([{"relative_offset": -600}]);
            }
            if index == 1 {
                fields["attendees"] = json!([{"name": "Sam Example", "url": "mailto:sam@example.com", "status": 2, "role": 1, "type": 1}]);
            }
            d.put(
                "calendar.event",
                &format!("demo-cal-event-{day}-{index}"),
                fields,
            );
        }
        if day % 7 == 0 {
            d.put(
                "calendar.event",
                &format!("demo-cal-day-{day}"),
                json!({
                    "calendar_id": reference("calendar.calendar", "demo-cal-family"),
                    "title": "A day together",
                    "start": date_only(date),
                    "end": date_only(date + 86400),
                    "all_day": true,
                    "time_zone": null,
                    "status": 1,
                    "availability": 0,
                    "is_exception": false,
                    "recurrence": [],
                    "alarms": [],
                    "attendees": [],
                }),
            );
        }
    }
    d.put(
        "calendar.event",
        "demo-cal-trip",
        json!({
            "calendar_id": reference("calendar.calendar", "demo-cal-personal"),
            "title": "Weekend away",
            "start": date_only(midnight + 4 * 86400),
            "end": date_only(midnight + 7 * 86400),
            "all_day": true,
            "time_zone": null,
            "status": 1,
            "location": "By the sea",
            "notes": "Tickets and a good book.",
            "recurrence": [],
            "alarms": [],
            "attendees": [],
        }),
    );
    d.put(
        "calendar.event",
        "demo-cal-overlap",
        json!({
            "calendar_id": reference("calendar.calendar", "demo-cal-work"),
            "title": "Design review",
            "start": ts(midnight + 14 * 3600 + 1800),
            "end": ts(midnight + 16 * 3600),
            "all_day": false,
            "time_zone": "America/New_York",
            "status": 2,
            "location": "Studio",
            "recurrence": [],
            "alarms": [],
            "attendees": [],
        }),
    );
}
