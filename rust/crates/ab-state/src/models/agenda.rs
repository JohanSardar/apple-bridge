use super::{Connection, ErrorCode, Result, Value, Visibility, b, data, json, list, s, visible};

pub(super) fn agenda(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let start = s(query, "start")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    let end = s(query, "end")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    if end <= start {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = Vec::new();
    if visible(visibility, "calendar") {
        for record in list(
            db,
            "SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0",
            &[],
        )? {
            let fields = data(&record)?;
            if !s(query, "calendar_id").is_empty()
                && s(&fields, "calendar_id") != s(query, "calendar_id")
            {
                continue;
            }
            let overlaps = if b(&fields, "all_day") {
                let start_day = &s(query, "start")[..10];
                let raw_end = s(query, "end");
                let mut end_day = raw_end[..10]
                    .parse::<jiff::civil::Date>()
                    .map_err(|_| ErrorCode::Invalid)?;
                let local_time = raw_end[11..]
                    .split(['Z', '+', '-'])
                    .next()
                    .ok_or(ErrorCode::Invalid)?
                    .parse::<jiff::civil::Time>()
                    .map_err(|_| ErrorCode::Invalid)?;
                if local_time != jiff::civil::Time::midnight() {
                    end_day = end_day
                        .checked_add(jiff::Span::new().days(1))
                        .map_err(|_| ErrorCode::Invalid)?;
                }
                s(&fields, "start") < end_day.to_string().as_str() && s(&fields, "end") > start_day
            } else {
                match (
                    s(&fields, "start").parse::<jiff::Timestamp>(),
                    s(&fields, "end").parse::<jiff::Timestamp>(),
                ) {
                    (Ok(a), Ok(b)) => a < end && b > start,
                    _ => false,
                }
            };
            if overlaps {
                out.push((s(&fields, "start").to_owned(), record));
            }
        }
    }
    out.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.id.as_str().cmp(right.1.id.as_str()))
    });
    Ok(json!({"records":out.into_iter().map(|(_,record)|record).collect::<Vec<_>>()}))
}
