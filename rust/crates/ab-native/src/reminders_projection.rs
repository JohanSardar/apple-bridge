use crate::reminders_store::{ListRow, ReminderRow, Scalar, Snapshot};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

const FIELDS: [&str; 28] = [
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
    "badge_emblem",
    "is_group",
    "display_order",
    "pinned",
    "pinned_at",
    "sorting_style",
    "categorize_groceries",
    "grocery_locale",
    "sharing_status",
    "shared_owner_name",
    "smart_list_type",
    "smart_filter",
    "sections",
    "store_color",
    "store_status",
    "store_coverage",
];

const REMINDER_FIELDS: [&str; 12] = [
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
];

const LIST_FIELDS: [&str; 16] = [
    "store_identifier",
    "parent_id",
    "badge_emblem",
    "is_group",
    "display_order",
    "pinned",
    "pinned_at",
    "sorting_style",
    "categorize_groceries",
    "grocery_locale",
    "sharing_status",
    "shared_owner_name",
    "smart_list_type",
    "smart_filter",
    "sections",
    "store_color",
];

const REFERENCE_EPOCH: f64 = 978_307_200.0;
const LIMIT: usize = 16_777_216;

pub type ColumnSets = BTreeMap<String, BTreeSet<String>>;

#[must_use]
pub fn base_data(data: &Value) -> Value {
    let mut result = data.as_object().cloned().unwrap_or_default();
    for field in FIELDS {
        result.remove(field);
    }
    Value::Object(result)
}

#[must_use]
pub fn base_observation(observation: &Value) -> Value {
    let mut result = observation.clone();
    if !observation["data"].is_null() {
        result["data"] = base_data(&observation["data"]);
    }
    result
}

#[must_use]
pub fn coverage(status: &str, reason: &str) -> Value {
    if reason.is_empty() {
        json!({"status": status})
    } else {
        json!({"status": status, "reason": reason})
    }
}

#[must_use]
pub fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}

fn date(value: Option<f64>) -> Value {
    value
        .and_then(|seconds| crate::capture::timestamp(seconds + REFERENCE_EPOCH).ok())
        .map_or(Value::Null, Value::String)
}

fn text(value: Option<&String>) -> Value {
    value.map_or(Value::Null, |value| Value::String(value.clone()))
}

fn number(value: Option<f64>) -> Value {
    value.map_or(Value::Null, |value| json!(value))
}

#[allow(clippy::cast_precision_loss)]
fn integer(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |value| json!(value as f64))
}

fn boolean(value: Option<bool>) -> Value {
    value.map_or(Value::Null, Value::Bool)
}

fn scalar(value: Option<&Scalar>) -> Value {
    match value {
        Some(Scalar::Integer(value)) => integer(Some(*value)),
        Some(Scalar::Real(value)) => json!(value),
        Some(Scalar::Text(value)) => json!(value),
        None => Value::Null,
    }
}

#[must_use]
pub fn reminder_identifier(row: Option<&ReminderRow>) -> Option<String> {
    let row = row?;
    let key = row.calendar_item_identifier.clone()?;
    (Some(&key) == row.identity.identifier().as_ref()).then_some(key)
}

#[must_use]
pub fn unavailable(collection: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("store_status".into(), json!("unavailable"));
    let mut covered = Map::new();
    let keys: &[&str] = if collection == "reminders.reminder" {
        &REMINDER_FIELDS
    } else {
        &LIST_FIELDS
    };
    for key in keys {
        fields.insert((*key).to_owned(), Value::Null);
        covered.insert(
            (*key).to_owned(),
            coverage("unavailable", "unique_exact_store_identity_unavailable"),
        );
    }
    fields.insert("store_coverage".into(), Value::Object(covered));
    Value::Object(fields)
}

#[must_use]
pub fn parse_uuid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let dash = [8, 13, 18, 23].contains(&index);
        if dash != (*byte == b'-') || (!dash && !byte.is_ascii_hexdigit()) {
            return None;
        }
    }
    Some(value.to_ascii_uppercase())
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManualOrder {
    Supported(BTreeMap<String, usize>),
    Unsupported,
}

impl ManualOrder {
    #[must_use]
    pub fn decode(data: Option<&[u8]>) -> Self {
        let Some(data) = data.filter(|data| data.len() <= LIMIT) else {
            return Self::Unsupported;
        };
        let Ok(values) = serde_json::from_slice::<Vec<String>>(data) else {
            return Self::Unsupported;
        };
        let mut positions = BTreeMap::new();
        for (index, value) in values.iter().enumerate() {
            let Some(uuid) = parse_uuid(value) else {
                return Self::Unsupported;
            };
            if positions.insert(uuid, index).is_some() {
                return Self::Unsupported;
            }
        }
        Self::Supported(positions)
    }

    #[must_use]
    pub fn changed_identifiers(&self, previous: &Self) -> Option<BTreeSet<String>> {
        match (previous, self) {
            (Self::Unsupported, Self::Unsupported) => Some(BTreeSet::new()),
            (Self::Supported(before), Self::Supported(after)) => {
                let mut changed = BTreeSet::new();
                for (identifier, position) in before {
                    if after.get(identifier) != Some(position) {
                        changed.insert(identifier.clone());
                    }
                }
                for (identifier, position) in after {
                    if before.get(identifier) != Some(position) {
                        changed.insert(identifier.clone());
                    }
                }
                Some(changed)
            }
            _ => None,
        }
    }
}

#[must_use]
pub fn tag_names(reminder: i64, snapshot: &Snapshot) -> Option<Vec<String>> {
    let mut names = BTreeSet::new();
    for object in snapshot.objects.values() {
        let Some(label) = object.hashtag_label else {
            continue;
        };
        let owners: BTreeSet<i64> = object
            .reminders
            .iter()
            .flatten()
            .copied()
            .filter(|owner| *owner != 0)
            .collect();
        if !owners.contains(&reminder) {
            continue;
        }
        let name = snapshot.labels.get(&label).and_then(|row| row.name.clone());
        match name {
            Some(name) if owners.len() == 1 && !name.is_empty() => {
                names.insert(name);
            }
            _ => return None,
        }
    }
    Some(names.into_iter().collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sections {
    pub order: Vec<String>,
    pub membership: BTreeMap<String, String>,
}

fn plain(value: &crate::plist::Value) -> Value {
    use crate::plist::Value as Plist;
    match value {
        Plist::Bool(value) => Value::Bool(*value),
        Plist::Integer(value) => json!(value),
        Plist::Real(value) => json!(value),
        Plist::String(value) => json!(value),
        Plist::Data(bytes) if bytes.len() == 16 => json!(crate::reminders_store::uuid_text(bytes)),
        Plist::Array(items) => Value::Array(items.iter().map(plain).collect()),
        Plist::Dictionary(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), plain(item)))
                .collect(),
        ),
        _ => Value::Null,
    }
}

fn object(data: Option<&[u8]>) -> Option<Value> {
    let data = data.filter(|data| data.len() <= LIMIT)?;
    if let Ok(value) = serde_json::from_slice::<Value>(data)
        && (value.is_array() || value.is_object())
    {
        return Some(value);
    }
    if let Ok(value) = crate::plist::decode(data) {
        let archived = value
            .dictionary()
            .is_some_and(|map| map.contains_key("$archiver"));
        if !archived {
            return Some(plain(&value));
        }
    }
    crate::plist::archive_root(data).ok().map(|value| plain(&value))
}

fn uuid(value: &Value) -> Option<String> {
    parse_uuid(value.as_str()?)
}

impl Sections {
    #[must_use]
    pub fn decode(order: Option<&[u8]>, membership: Option<&[u8]>) -> Option<Self> {
        let raw = object(order)?;
        let values: Vec<Value> = if let Some(wrapper) = raw.as_object() {
            if wrapper.get("minimumSupportedVersion").and_then(Value::as_f64) != Some(20_230_430.0)
            {
                return None;
            }
            wrapper.get("orderedIdentifiers")?.as_array()?.clone()
        } else {
            raw.as_array()?.clone()
        };
        let identifiers: Vec<String> = values.iter().filter_map(uuid).collect();
        let distinct: BTreeSet<&String> = identifiers.iter().collect();
        if identifiers.len() != values.len() || distinct.len() != identifiers.len() {
            return None;
        }
        let mut members: BTreeMap<String, String> = BTreeMap::new();
        let mut insert = |reminder: &Value, section: &Value| -> bool {
            let (Some(reminder), Some(section)) = (uuid(reminder), uuid(section)) else {
                return false;
            };
            if !identifiers.contains(&section) || members.contains_key(&reminder) {
                return false;
            }
            members.insert(reminder, section);
            true
        };
        if membership.is_none() {
            return Some(Self {
                order: identifiers,
                membership: BTreeMap::new(),
            });
        }
        let decoded = object(membership);
        if let Some(wrapper) = decoded
            .as_ref()
            .and_then(Value::as_object)
            .filter(|wrapper| wrapper.contains_key("memberships"))
        {
            if wrapper.get("minimumSupportedVersion").and_then(Value::as_f64) != Some(20_230_430.0)
            {
                return None;
            }
            for entry in wrapper.get("memberships")?.as_array()? {
                let entry = entry.as_object()?;
                if !insert(entry.get("memberID")?, entry.get("groupID")?) {
                    return None;
                }
            }
        } else if let Some(dictionary) = decoded.as_ref().and_then(Value::as_object) {
            for (key, value) in dictionary {
                let key = Value::String(key.clone());
                if let Some(reminders) = value.as_array() {
                    if !reminders.iter().all(|reminder| insert(reminder, &key)) {
                        return None;
                    }
                } else if !insert(&key, value) {
                    return None;
                }
            }
        } else {
            let pairs = decoded
                .as_ref()
                .and_then(Value::as_array)
                .filter(|pairs| pairs.len() % 2 == 0)?;
            for pair in pairs.chunks(2) {
                if let Some(reminders) = pair[1].as_array() {
                    if !reminders.iter().all(|reminder| insert(reminder, &pair[0])) {
                        return None;
                    }
                } else if !insert(&pair[0], &pair[1]) {
                    return None;
                }
            }
        }
        Some(Self {
            order: identifiers,
            membership: members,
        })
    }

    #[must_use]
    pub fn validated(list: &ListRow, snapshot: &Snapshot) -> Option<Self> {
        let sections: Vec<_> = snapshot
            .sections
            .values()
            .filter(|section| section.list == Some(list.identity.key))
            .collect();
        if sections.is_empty() && list.section_order.is_none() && list.section_membership.is_none()
        {
            return Some(Self {
                order: Vec::new(),
                membership: BTreeMap::new(),
            });
        }
        let mut order = list.section_order.clone();
        if order.is_none() {
            let dates: Vec<f64> = sections.iter().filter_map(|section| section.created).collect();
            let distinct: BTreeSet<u64> = dates.iter().map(|date| date.to_bits()).collect();
            if dates.len() != sections.len()
                || !dates.iter().all(|date| date.is_finite())
                || distinct.len() != dates.len()
            {
                return None;
            }
            let mut sorted = sections.clone();
            sorted.sort_by(|left, right| {
                left.created
                    .partial_cmp(&right.created)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let identifiers: Vec<String> = sorted
                .iter()
                .filter_map(|section| section.identity.identifier())
                .collect();
            if identifiers.len() != sections.len() {
                return None;
            }
            order = serde_json::to_vec(&identifiers).ok();
        }
        let decoded = Self::decode(order.as_deref(), list.section_membership.as_deref())?;
        let identities: Vec<String> = sections
            .iter()
            .filter_map(|section| section.identity.identifier().as_deref().and_then(parse_uuid))
            .collect();
        let identity_set: BTreeSet<&String> = identities.iter().collect();
        let order_set: BTreeSet<&String> = decoded.order.iter().collect();
        if identities.len() != sections.len() || identity_set != order_set {
            return None;
        }
        let reminders: BTreeSet<String> = snapshot
            .reminders
            .values()
            .filter(|row| row.list == Some(list.identity.key))
            .filter_map(|row| reminder_identifier(Some(row)).as_deref().and_then(parse_uuid))
            .collect();
        decoded
            .membership
            .keys()
            .all(|key| reminders.contains(key))
            .then_some(decoded)
    }

    #[must_use]
    pub fn fields(list: &ListRow, snapshot: &Snapshot) -> Option<Value> {
        let decoded = Self::validated(list, snapshot)?;
        let mut by_id = BTreeMap::new();
        for section in snapshot
            .sections
            .values()
            .filter(|section| section.list == Some(list.identity.key))
        {
            let id = section.identity.identifier().as_deref().and_then(parse_uuid)?;
            if by_id.insert(id, section).is_some() {
                return None;
            }
        }
        Some(Value::Array(
            decoded
                .order
                .iter()
                .enumerate()
                .map(|(index, id)| {
                    let members: BTreeSet<&String> = decoded
                        .membership
                        .iter()
                        .filter(|(_, section)| *section == id)
                        .map(|(reminder, _)| reminder)
                        .collect();
                    json!({
                        "id": id,
                        "title": by_id.get(id).and_then(|section| section.name.clone()).unwrap_or_default(),
                        "kind": "store",
                        "order": index_number(index),
                        "reminder_ids": members.into_iter().map(|reminder| reference("reminders.reminder", reminder)).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        ))
    }
}

#[allow(clippy::cast_precision_loss)]
fn index_number(index: usize) -> f64 {
    index as f64
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn reminder(
    row: &ReminderRow,
    snapshot: &Snapshot,
    columns: &BTreeSet<String>,
    manual_order: &ManualOrder,
    sections: Option<&Sections>,
    tag_columns: &ColumnSets,
) -> Value {
    let parent = row
        .parent
        .and_then(|key| snapshot.reminders.get(&key))
        .and_then(|parent| reminder_identifier(Some(parent)));
    let mut fields = Map::new();
    fields.insert(
        "store_identifier".into(),
        row.identity
            .identifier()
            .map_or(Value::Null, Value::String),
    );
    fields.insert("flagged".into(), boolean(row.flagged));
    fields.insert(
        "parent_id".into(),
        parent
            .as_ref()
            .map_or(Value::Null, |parent| reference("reminders.reminder", parent)),
    );
    fields.insert("created_at".into(), date(row.created));
    fields.insert("store_status".into(), json!("partial"));
    let mut covered = Map::new();
    for key in REMINDER_FIELDS {
        if !fields.contains_key(key) {
            fields.insert(key.to_owned(), Value::Null);
            covered.insert(
                key.to_owned(),
                coverage("unsupported", "store_decoder_unverified"),
            );
        }
    }
    for (field, column) in [
        ("store_identifier", "ZIDENTIFIER"),
        ("flagged", "ZFLAGGED"),
        ("parent_id", "ZPARENTREMINDER"),
        ("created_at", "ZCREATIONDATE"),
    ] {
        covered.insert(
            field.to_owned(),
            coverage(if columns.contains(column) { "complete" } else { "unsupported" }, ""),
        );
    }
    let own = row.identity.identifier().as_deref().and_then(parse_uuid);
    if let (ManualOrder::Supported(positions), Some(identity)) = (manual_order, own.as_ref()) {
        let position = positions.get(identity);
        fields.insert(
            "manual_order".into(),
            position.map_or(Value::Null, |position| json!(index_number(*position))),
        );
        covered.insert(
            "manual_order".into(),
            coverage(
                if position.is_none() { "partial" } else { "complete" },
                if position.is_none() { "reminder_not_in_ordering" } else { "" },
            ),
        );
    }
    if let (Some(sections), Some(id)) = (
        sections,
        reminder_identifier(Some(row)).as_deref().and_then(parse_uuid),
    ) {
        fields.insert(
            "section_id".into(),
            sections
                .membership
                .get(&id)
                .map_or(Value::Null, |section| json!(section)),
        );
        covered.insert("section_id".into(), coverage("complete", ""));
    }
    let object_columns = tag_columns.get("ZREMCDOBJECT");
    if ["ZHASHTAGLABEL", "ZREMINDER3"]
        .iter()
        .all(|column| object_columns.is_some_and(|set| set.contains(*column)))
        && tag_columns
            .get("ZREMCDHASHTAGLABEL")
            .is_some_and(|set| set.contains("ZNAME"))
        && let Some(tags) = tag_names(row.identity.key, snapshot)
    {
        fields.insert("tags".into(), json!(tags));
        covered.insert("tags".into(), coverage("complete", ""));
    }
    if row.flagged.is_none() && columns.contains("ZFLAGGED") {
        covered.insert("flagged".into(), coverage("unavailable", ""));
    }
    if let Some(key) = row.parent
        && key != 0
        && parent.is_none()
    {
        covered.insert(
            "parent_id".into(),
            coverage("partial", "parent_identity_unavailable"),
        );
    }
    fields.insert("store_coverage".into(), Value::Object(covered));
    Value::Object(fields)
}

#[must_use]
pub fn list(row: &ListRow, snapshot: &Snapshot, columns: &BTreeSet<String>) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "store_identifier".into(),
        row.identity.identifier().map_or(Value::Null, Value::String),
    );
    fields.insert("badge_emblem".into(), text(row.emblem.as_ref()));
    fields.insert("display_order".into(), number(row.display_order));
    fields.insert("sorting_style".into(), scalar(row.sorting_style.as_ref()));
    fields.insert("grocery_locale".into(), text(row.grocery_locale.as_ref()));
    fields.insert("sharing_status".into(), integer(row.sharing_status));
    fields.insert("shared_owner_name".into(), text(row.shared_owner.as_ref()));
    fields.insert("smart_list_type".into(), scalar(row.smart_type.as_ref()));
    fields.insert("is_group".into(), boolean(row.is_group));
    fields.insert("pinned".into(), boolean(row.pinned));
    fields.insert("categorize_groceries".into(), boolean(row.groceries));
    fields.insert("pinned_at".into(), date(row.pinned_date));
    fields.insert("store_status".into(), json!("partial"));
    let candidates: BTreeSet<i64> = [row.parent, row.alternate_parent]
        .into_iter()
        .flatten()
        .filter(|candidate| *candidate != 0)
        .collect();
    let parent = if candidates.len() == 1 {
        candidates
            .iter()
            .next()
            .and_then(|key| snapshot.lists.get(key))
            .and_then(|parent| parent.identity.identifier())
    } else {
        None
    };
    fields.insert(
        "parent_id".into(),
        parent
            .as_ref()
            .map_or(Value::Null, |parent| reference("reminders.list", parent)),
    );
    let mut covered = Map::new();
    for key in LIST_FIELDS {
        if !fields.contains_key(key) {
            fields.insert(key.to_owned(), Value::Null);
            covered.insert(
                key.to_owned(),
                coverage("unsupported", "store_decoder_unverified"),
            );
        }
    }
    for (field, column) in [
        ("store_identifier", "ZIDENTIFIER"),
        ("badge_emblem", "ZBADGEEMBLEM"),
        ("display_order", "ZDADISPLAYORDER"),
        ("sorting_style", "ZSORTINGSTYLE"),
        ("grocery_locale", "ZGROCERYLOCALEID"),
        ("sharing_status", "ZSHARINGSTATUS"),
        ("shared_owner_name", "ZSHAREDOWNERNAME"),
        ("smart_list_type", "ZSMARTLISTTYPE"),
        ("is_group", "ZISGROUP"),
        ("pinned", "ZISPINNEDBYCURRENTUSER"),
        ("categorize_groceries", "ZSHOULDCATEGORIZEGROCERYITEMS"),
        ("pinned_at", "ZPINNEDDATE"),
    ] {
        covered.insert(
            field.to_owned(),
            coverage(if columns.contains(column) { "complete" } else { "unsupported" }, ""),
        );
    }
    if let Some(sections) = Sections::fields(row, snapshot) {
        fields.insert("sections".into(), sections);
        covered.insert("sections".into(), coverage("complete", ""));
    }
    let parent_status = if candidates.len() > 1 {
        "unsupported"
    } else if !candidates.is_empty() && parent.is_none() {
        "partial"
    } else if columns.contains("ZPARENTLIST") || columns.contains("ZPARENTLIST1") {
        "complete"
    } else {
        "unsupported"
    };
    covered.insert("parent_id".into(), coverage(parent_status, ""));
    fields.insert("store_coverage".into(), Value::Object(covered));
    Value::Object(fields)
}
