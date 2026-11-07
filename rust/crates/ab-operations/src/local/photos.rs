use super::{Code, Error, NativeQuery, Value, descriptor_for};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Previews(Mutex<BTreeMap<String, (Instant, Box<RawValue>)>>);

impl std::fmt::Debug for Previews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhotosPreviews(<private>)")
    }
}

impl Previews {
    pub(super) fn remember(&self, id: &str, args: &RawValue) -> Result<(), Error> {
        let mut previews = self.0.lock().map_err(|_| Code::Unavailable)?;
        previews.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
        let bytes: usize = previews.values().map(|(_, value)| value.get().len()).sum();
        let replaced = previews.get(id).map_or(0, |(_, value)| value.get().len());
        if previews.len() >= 128 && !previews.contains_key(id)
            || args.get().len() > 1 << 20
            || bytes - replaced + args.get().len() > 16 << 20
        {
            return Err(Code::Capacity.into());
        }
        previews.insert(id.into(), (Instant::now(), args.to_owned()));
        Ok(())
    }

    pub(super) fn get(&self, id: &str) -> Result<Box<RawValue>, Error> {
        let mut previews = self.0.lock().map_err(|_| Code::Unavailable)?;
        previews.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
        previews
            .get(id)
            .map(|(_, args)| args.clone())
            .ok_or_else(|| Code::Unavailable.into())
    }

    pub(super) fn remove(&self, id: &str) -> Result<(), Error> {
        self.0.lock().map_err(|_| Code::Unavailable)?.remove(id);
        Ok(())
    }
}

impl super::LocalGateway {
    pub(crate) fn photos_preview(
        &self,
        mut record: super::CommandRecord,
    ) -> Result<super::CommandRecord, Error> {
        if mutation(&record.command.op) {
            let args = self.photos_previews.get(&record.command.id)?;
            let canonical = self.codec.json(args.get().as_bytes())?;
            let stored: Value =
                serde_json::from_str(record.command.args.get()).map_err(|_| Code::Corrupt)?;
            if stored["request_digest"] != self.codec.fingerprint(&canonical) {
                return Err(Code::Corrupt.into());
            }
            record.command.args = args;
        }
        Ok(record)
    }
}

pub(crate) fn mutation(operation: &str) -> bool {
    matches!(
        operation,
        "photos.favorite"
            | "photos.album_create"
            | "photos.album_rename"
            | "photos.album_add"
            | "photos.album_remove"
            | "photos.album_delete"
    )
}

pub(super) fn validate(query: &NativeQuery, args: &Value) -> Result<(), Error> {
    if !query.op.starts_with("photos.") {
        return Ok(());
    }
    if mutation(&query.op) && query.id.is_empty() {
        return Err(Code::Invalid.into());
    }
    let descriptor = descriptor_for(&query.op)?;
    validate_value(args, &descriptor.input["properties"]["args"])?;
    if query.op == "photos.search" {
        let start = args.get("start").map(|v| {
            v.as_str()
                .ok_or(Code::Invalid)?
                .parse::<jiff::Timestamp>()
                .map_err(|_| Code::Invalid)
        });
        let end = args.get("end").map(|v| {
            v.as_str()
                .ok_or(Code::Invalid)?
                .parse::<jiff::Timestamp>()
                .map_err(|_| Code::Invalid)
        });
        let (start, end) = (start.transpose()?, end.transpose()?);
        if start.zip(end).is_some_and(|(start, end)| start >= end)
            || args.get("radius_m").is_some() && args.get("near").is_none()
        {
            return Err(Code::Invalid.into());
        }
    }
    if query.op == "photos.fetch" {
        let fields = args.as_object().ok_or(Code::Invalid)?;
        let allowed: &[&str] = if fields.contains_key("transfer_id") {
            &["transfer_id", "offset", "length"]
        } else {
            if !fields.contains_key("asset_id") {
                return Err(Code::Invalid.into());
            }
            &[
                "asset_id",
                "resource_index",
                "resource_key",
                "network_access",
                "max_bytes",
            ]
        };
        if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(Code::Invalid.into());
        }
    }
    Ok(())
}

fn validate_value(value: &Value, schema: &Value) -> Result<(), Error> {
    let valid = match schema["type"].as_str() {
        Some("object") => {
            let object = value.as_object().ok_or(Code::Invalid)?;
            let properties = schema["properties"].as_object().ok_or(Code::Corrupt)?;
            if object.keys().any(|key| !properties.contains_key(key))
                || schema["required"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|key| !object.contains_key(key.as_str().unwrap_or_default()))
            {
                return Err(Code::Invalid.into());
            }
            for (key, child) in object {
                validate_value(child, &properties[key])?;
            }
            true
        }
        Some("array") => {
            let array = value.as_array().ok_or(Code::Invalid)?;
            let count = array.len() as u64;
            if count < schema["minItems"].as_u64().unwrap_or(0)
                || count > schema["maxItems"].as_u64().unwrap_or(u64::MAX)
            {
                return Err(Code::Invalid.into());
            }
            for (index, child) in array.iter().enumerate() {
                validate_value(child, &schema["items"])?;
                if schema["uniqueItems"] == true && array[..index].contains(child) {
                    return Err(Code::Invalid.into());
                }
            }
            true
        }
        Some("string") => value.as_str().is_some_and(|text| {
            !text.trim().is_empty()
                && text.chars().count() as u64 >= schema["minLength"].as_u64().unwrap_or(0)
                && text.chars().count() as u64 <= schema["maxLength"].as_u64().unwrap_or(u64::MAX)
        }),
        Some("boolean") => value.is_boolean(),
        Some("number" | "integer") => value.as_f64().is_some_and(|number| {
            number.is_finite()
                && (schema["type"] != "integer" || value.as_u64().is_some())
                && schema["minimum"].as_f64().is_none_or(|min| number >= min)
                && schema["exclusiveMinimum"]
                    .as_f64()
                    .is_none_or(|min| number > min)
                && schema["maximum"].as_f64().is_none_or(|max| number <= max)
        }),
        _ => return Err(Code::Corrupt.into()),
    };
    if !valid
        || schema["enum"]
            .as_array()
            .is_some_and(|choices| !choices.contains(value))
    {
        return Err(Code::Invalid.into());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn previews_expire_redact_debug_and_bound_entries_and_bytes() {
        let previews = Previews::default();
        let args = serde_json::value::to_raw_value(&"synthetic-private-preview").expect("args");
        for index in 0..128 {
            previews
                .remember(&index.to_string(), &args)
                .expect("bounded entry");
        }
        assert_eq!(
            previews
                .remember("overflow", &args)
                .expect_err("capacity")
                .code,
            Code::Capacity
        );
        assert!(!format!("{previews:?}").contains("synthetic-private-preview"));
        previews.0.lock().expect("previews").insert(
            "0".into(),
            (
                Instant::now()
                    .checked_sub(Duration::from_secs(600))
                    .expect("past instant"),
                args,
            ),
        );
        assert_eq!(
            previews.get("0").expect_err("expired").code,
            Code::Unavailable
        );
        let previews = Previews::default();
        let args = serde_json::value::to_raw_value(&"x".repeat((1 << 20) - 2)).expect("args");
        for index in 0..16 {
            previews
                .remember(&index.to_string(), &args)
                .expect("bounded bytes");
        }
        assert_eq!(
            previews
                .remember("overflow", &args)
                .expect_err("byte capacity")
                .code,
            Code::Capacity
        );
        previews
            .remember("0", &args)
            .expect("replacement has no extra cost");
    }
}
