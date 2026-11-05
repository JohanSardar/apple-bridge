use super::*;

pub(super) fn validate_query(operation: &str, query: &Value) -> Result<()> {
    let allowed = match operation {
        "messages.chats" => "participants filter limit after",
        "messages.thread" => "thread_originator_id chat_id anchor before after limit",
        "messages.collection" => "chat_id kind after limit",
        "messages.search" => "text kind from_handle_id from_contact_id unread after limit",
        "messages.handles" => "ids",
        "reminders.lists" => "time_zone",
        "reminders.items" => "time_zone list smart tag show_completed sort",
        "calendar.agenda" => "start end calendar_id",
        "calendar.range" => "start end calendars time_zone text",
        "contacts.lookup" => "q limit name handle",
        "contacts.list" => "group sort display_order",
        "notes.list" => "folder sort time_zone",
        "health.summary" => "period date time_zone device_id",
        "replica.status" | "sources.status" => "",
        _ => return Err(ErrorCode::Unsupported.into()),
    };
    let fields = query.as_object().ok_or(ErrorCode::Invalid)?;
    for (key, value) in fields {
        if !allowed.split_whitespace().any(|a| a == key) {
            return Err(ErrorCode::Invalid.into());
        }
        if value.is_null() {
            continue;
        }
        let valid = match key.as_str() {
            "limit" => value.as_i64().is_some(),
            "unread" | "show_completed" => value.is_boolean(),
            "participants" | "ids" | "calendars" => value
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v.is_string() || v.is_null())),
            _ => value.is_string(),
        };
        if !valid {
            return Err(ErrorCode::Invalid.into());
        }
    }
    Ok(())
}
