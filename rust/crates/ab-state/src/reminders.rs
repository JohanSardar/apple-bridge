struct Tree<'a> {
    nodes: &'a BTreeMap<String, (Record, Value, String)>,
    children: HashMap<(String, String), Vec<String>>,
    media: HashMap<String, Vec<Record>>,
    sort: &'a str,
    zone: &'a jiff::tz::TimeZone,
    collator: icu_collator::CollatorBorrowed<'static>,
}
impl Tree<'_> {
    fn build(&self, parent: &str, group: &str) -> Result<Vec<Value>> {
        let mut children = self
            .children
            .get(&(parent.into(), group.into()))
            .cloned()
            .unwrap_or_default();
        children.sort_by(|left, right| {
            let a = &self.nodes[left].1;
            let b = &self.nodes[right].1;
            let order = match self.sort {
                "priority" => {
                    let rank = |d: &Value| {
                        let p = d["priority"].as_i64().unwrap_or(0);
                        if p == 0 { 10 } else { p }
                    };
                    rank(a).cmp(&rank(b))
                }
                "title" => self.collator.compare(s(a, "title"), s(b, "title")),
                "due" | "created" => {
                    let key = if self.sort == "due" {
                        "due"
                    } else {
                        "created_at"
                    };
                    let first = sort_time(s(a, key), self.zone);
                    let second = sort_time(s(b, key), self.zone);
                    first
                        .is_none()
                        .cmp(&second.is_none())
                        .then(first.cmp(&second))
                }
                _ => a["manual_order"]
                    .is_null()
                    .cmp(&b["manual_order"].is_null())
                    .then(
                        a["manual_order"]
                            .as_f64()
                            .unwrap_or(0.0)
                            .total_cmp(&b["manual_order"].as_f64().unwrap_or(0.0)),
                    ),
            };
            order.then(left.cmp(right))
        });
        let mut out = Vec::new();
        for id in children {
            let record = &self.nodes[&id].0;
            let media = self.media.get(&id).cloned().unwrap_or_default();
            out.push(
                json!({"reminder":record,"subtasks":self.build(&id,group)?,"attachments":media}),
            );
        }
        Ok(out)
    }
}
fn sort_time(raw: &str, zone: &jiff::tz::TimeZone) -> Option<jiff::Timestamp> {
    raw.parse::<jiff::Timestamp>().ok().or_else(|| {
        raw.parse::<jiff::civil::Date>()
            .ok()?
            .at(0, 0, 0, 0)
            .to_zoned(zone.clone())
            .ok()
            .map(|t| t.timestamp())
    })
}
use crate::projection::{b, s};
use crate::{ErrorCode, Record, Result, Visibility, records};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
fn data(record: &Record) -> Result<Value> {
    Ok(serde_json::from_str(record.data.get())?)
}
fn zone(query: &Value) -> Result<jiff::tz::TimeZone> {
    let name = s(query, "time_zone");
    if name.is_empty() || name == "Local" {
        return Err(ErrorCode::Invalid.into());
    }
    jiff::tz::TimeZone::get(name).map_err(|_| ErrorCode::Invalid.into())
}
fn due(fields: &Value, z: &jiff::tz::TimeZone) -> Result<String> {
    let raw = s(fields, "due");
    if raw.is_empty() {
        return Ok(String::new());
    }
    if let Ok(day) = raw.parse::<jiff::civil::Date>() {
        return Ok(day.to_string());
    }
    raw.parse::<jiff::Timestamp>()
        .map(|t| t.to_zoned(z.clone()).date().to_string())
        .map_err(|_| ErrorCode::Unsupported.into())
}
fn assigned(fields: &Value) -> (bool, bool) {
    if fields["assignment"].is_null() {
        return (
            false,
            s(&fields["store_coverage"]["assignment"], "status") == "complete",
        );
    }
    let a = &fields["assignment"]["assignee"];
    (
        b(a, "is_me"),
        !s(a, "id").is_empty() && a["is_me"].is_boolean(),
    )
}
fn custom(fields: &Value) -> bool {
    ["smart_list_type", "smart_filter"]
        .iter()
        .any(|k| !fields[k].is_null() && fields[k] != "")
}
fn all(db: &Connection, c: &str) -> Result<Vec<Record>> {
    records(
        db,
        "SELECT payload FROM records WHERE collection=? AND deleted=0 ORDER BY id",
        &[&c],
    )
}
fn section(kind: &str, id: &str, title: &str, order: &Value) -> Value {
    json!({"kind":kind,"id":id,"title":title,"order":order})
}
fn section_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    (s(a, "kind") == "none")
        .cmp(&(s(b, "kind") == "none"))
        .then(
            a["order"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&b["order"].as_f64().unwrap_or(0.0)),
        )
        .then(s(a, "id").cmp(s(b, "id")))
}
#[allow(clippy::too_many_lines)]
pub(crate) fn lists(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let z = zone(query)?;
    let mut out = json!({"smart":[],"lists":[],"tags":[]});
    if visibility.get("reminders") != Some(&true) {
        return Ok(out);
    }
    let today = jiff::Timestamp::now()
        .to_zoned(z.clone())
        .date()
        .to_string();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut unsupported = HashSet::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut lc: HashMap<String, usize> = HashMap::new();
    let mut tags_unknown = false;
    let mut identity = false;
    for record in all(db, "reminders.reminder")? {
        let fields = data(&record)?;
        let (ass, known) = assigned(&fields);
        if !fields["assignment"].is_null() && known {
            identity = true;
        }
        if b(&fields, "completed") {
            *counts.entry("completed".into()).or_default() += 1;
            continue;
        }
        *counts.entry("all".into()).or_default() += 1;
        *lc.entry(s(&fields, "list_id").into()).or_default() += 1;
        if let Ok(day) = due(&fields, &z) {
            if !day.is_empty() {
                *counts.entry("scheduled".into()).or_default() += 1;
                if day <= today {
                    *counts.entry("today".into()).or_default() += 1;
                }
            }
        } else {
            unsupported.insert("today");
            unsupported.insert("scheduled");
        }
        if fields["flagged"].is_null() {
            unsupported.insert("flagged");
        } else if b(&fields, "flagged") {
            *counts.entry("flagged".into()).or_default() += 1;
        }
        if !known {
            unsupported.insert("assigned");
        } else if ass {
            *counts.entry("assigned".into()).or_default() += 1;
        }
        if let Some(ts) = fields["tags"].as_array() {
            let mut seen = HashSet::new();
            for t in ts {
                if let Some(t) = t.as_str()
                    && !t.is_empty()
                    && seen.insert(t)
                {
                    *tags.entry(t.into()).or_default() += 1;
                }
            }
        } else {
            tags_unknown = true;
        }
    }
    let smart: Vec<_> = [
        "today",
        "scheduled",
        "all",
        "flagged",
        "completed",
        "assigned",
    ]
    .iter()
    .map(|id| {
        if unsupported.contains(id) || *id == "assigned" && !identity {
            json!({"id":id,"status":"unsupported"})
        } else {
            json!({"id":id,"count":counts.get(*id).copied().unwrap_or(0)})
        }
    })
    .collect();
    let mut ls = Vec::new();
    for record in all(db, "reminders.list")? {
        let fields = data(&record)?;
        let mut sections = fields["sections"].as_array().cloned().unwrap_or_default();
        sections.sort_by(section_order);
        let mut l = json!({"list":record,"sections":sections,"group_id":fields["parent_id"],"order":fields["display_order"],"pinned":fields["pinned"]});
        if custom(&fields) {
            l["status"] = "unsupported".into();
        } else if !b(&fields, "is_group") {
            l["count"] = lc.get(record.id.as_str()).copied().unwrap_or(0).into();
        }
        ls.push(l);
    }
    ls.sort_by(|a, bv| {
        let ap = b(a, "pinned");
        let bp = b(bv, "pinned");
        bp.cmp(&ap)
            .then(a["order"].is_null().cmp(&bv["order"].is_null()))
            .then(
                a["order"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .total_cmp(&bv["order"].as_f64().unwrap_or(0.0)),
            )
            .then(s(&a["list"], "id").cmp(s(&bv["list"], "id")))
    });
    out["smart"] = smart.into();
    out["lists"] = ls.into();
    if !tags_unknown {
        out["tags"] = tags
            .into_iter()
            .map(|(name, count)| json!({"name":name,"count":count}))
            .collect::<Vec<_>>()
            .into();
    }
    Ok(out)
}
#[allow(clippy::too_many_lines)]
pub(crate) fn items(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let z = zone(query)?;
    let selectors = [s(query, "list"), s(query, "smart"), s(query, "tag")]
        .iter()
        .filter(|s| !s.is_empty())
        .count();
    let sort = s(query, "sort");
    let smart = s(query, "smart");
    if selectors != 1
        || !["", "manual", "due", "created", "priority", "title"].contains(&sort)
        || ![
            "",
            "today",
            "scheduled",
            "all",
            "flagged",
            "completed",
            "assigned",
        ]
        .contains(&smart)
    {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = json!({"sections":[],"count":0});
    if visibility.get("reminders") != Some(&true) {
        return Ok(out);
    }
    let lists = all(db, "reminders.list")?;
    let mut sections: BTreeMap<String, Value> = BTreeMap::new();
    if !s(query, "list").is_empty() {
        let record = lists
            .iter()
            .find(|record| record.id.as_str() == s(query, "list"))
            .ok_or(ErrorCode::NotFound)?;
        let fields = data(record)?;
        if custom(&fields) {
            return Err(ErrorCode::Unsupported.into());
        }
        if let Some(ss) = fields["sections"].as_array() {
            for sec in ss {
                if s(sec, "kind") == "store" && !s(sec, "id").is_empty() {
                    sections.insert(s(sec, "id").into(), sec.clone());
                }
            }
        }
    }
    let today = jiff::Timestamp::now()
        .to_zoned(z.clone())
        .date()
        .to_string();
    let mut identity = false;
    let mut nodes: BTreeMap<String, (Record, Value, String)> = BTreeMap::new();
    let rs = if s(query, "list").is_empty() {
        all(db, "reminders.reminder")?
    } else {
        records(
            db,
            "SELECT payload FROM records WHERE collection='reminders.reminder' AND deleted=0 AND json_extract(payload,'$.data.list_id')=? ORDER BY id",
            &[&s(query, "list")],
        )?
    };
    for record in rs {
        let fields = data(&record)?;
        let (ass, known) = assigned(&fields);
        identity |= !fields["assignment"].is_null() && known;
        let selected = if smart == "completed" {
            b(&fields, "completed")
        } else if b(&fields, "completed") && !b(query, "show_completed") {
            false
        } else if !s(query, "tag").is_empty() {
            let ts = fields["tags"].as_array().ok_or(ErrorCode::Unsupported)?;
            ts.iter().any(|t| t == s(query, "tag"))
        } else {
            match smart {
                "today" | "scheduled" => {
                    let day = due(&fields, &z)?;
                    !day.is_empty() && (smart == "scheduled" || day <= today)
                }
                "flagged" => fields["flagged"].as_bool().ok_or(ErrorCode::Unsupported)?,
                "assigned" => {
                    if !known {
                        return Err(ErrorCode::Unsupported.into());
                    }
                    ass
                }
                _ => true,
            }
        };
        if !selected {
            continue;
        }
        let sec = if !s(query, "list").is_empty() && sections.contains_key(s(&fields, "section_id"))
        {
            sections[s(&fields, "section_id")].clone()
        } else if smart == "scheduled" {
            let day = due(&fields, &z)?;
            section("date", &day, &day, &json!(0))
        } else if smart == "all" || smart == "completed" {
            let id = s(&fields, "list_id");
            let l = lists
                .iter()
                .find(|record| record.id.as_str() == id)
                .ok_or(ErrorCode::NotFound)?;
            let ld = data(l)?;
            let mut sec = section(
                "list",
                id,
                s(&ld, "title"),
                &ld.get("display_order").cloned().unwrap_or_else(|| json!(0)),
            );
            sec["list_id"] = id.into();
            sec
        } else {
            section("none", "none", "", &json!(0))
        };
        let group = s(&sec, "id").to_owned();
        sections.insert(group.clone(), sec);
        nodes.insert(record.id.as_str().into(), (record, fields, group));
    }
    if smart == "assigned" && !identity {
        return Err(ErrorCode::Unsupported.into());
    }
    let attachments = all(db, "reminders.attachment")?;
    let mut parents: HashMap<String, String> = HashMap::new();
    for (id, (_, fields, g)) in &nodes {
        if let Some((_, _, pg)) = nodes.get(s(fields, "parent_id"))
            && g == pg
        {
            parents.insert(id.clone(), s(fields, "parent_id").into());
        }
    }
    let mut done = HashSet::new();
    for id in nodes.keys() {
        let mut path = Vec::new();
        let mut next = id.clone();
        while !next.is_empty() && !done.contains(&next) {
            if let Some(pos) = path.iter().position(|p| p == &next) {
                if let Some(root) = path[pos..].iter().min() {
                    parents.remove(root);
                }
                break;
            }
            path.push(next.clone());
            next = parents.get(&next).cloned().unwrap_or_default();
        }
        done.extend(path);
    }
    let mut children: HashMap<(String, String), Vec<String>> = HashMap::new();
    for (id, (_, _, group)) in &nodes {
        children
            .entry((parents.get(id).cloned().unwrap_or_default(), group.clone()))
            .or_default()
            .push(id.clone());
    }
    let mut media: HashMap<String, Vec<Record>> = HashMap::new();
    for record in attachments {
        media
            .entry(s(&data(&record)?, "reminder_id").into())
            .or_default()
            .push(record);
    }
    let mut prefs = icu_collator::CollatorPreferences::default();
    prefs.numeric_ordering = Some(icu_collator::preferences::CollationNumericOrdering::True);
    let mut options = icu_collator::options::CollatorOptions::default();
    options.strength = Some(icu_collator::options::Strength::Secondary);
    let collator =
        icu_collator::Collator::try_new(prefs, options).map_err(|_| ErrorCode::Unavailable)?;
    let tree = Tree {
        nodes: &nodes,
        children,
        media,
        sort,
        zone: &z,
        collator,
    };
    let mut ss = Vec::new();
    for (id, sec) in sections {
        ss.push(json!({"section":sec,"items":tree.build("",&id)?}));
    }
    ss.sort_by(|a, b| section_order(&a["section"], &b["section"]));
    out["sections"] = ss.into();
    out["count"] = nodes.len().into();
    Ok(out)
}
