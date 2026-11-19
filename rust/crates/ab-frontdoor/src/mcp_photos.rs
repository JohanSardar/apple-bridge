use crate::{
    mcp_check::{Problem, from_error},
    mcp_facade::{Call, Outcome, Step, text},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

pub(crate) const SHOW_MAX_BYTES: u64 = 3_750_000;
pub(crate) const ATTACH_MAX_BYTES: u64 = 100 << 20;
pub(crate) const ATTACH_MAX_PHOTOS: usize = 10;
const CHUNK: u64 = 64 << 10;
const WAIT: Duration = Duration::from_secs(90);

pub(crate) struct PhotoFile {
    pub bytes: Vec<u8>,
    pub name: String,
}

const CURRENT_PHOTO: i64 = 5;
const ORIGINAL_PHOTO: i64 = 1;
const CURRENT_VIDEO: i64 = 6;
const ORIGINAL_VIDEO: i64 = 2;

impl Call<'_> {
    pub(crate) async fn photos_show(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let id = text(input, "asset_id").ok_or_else(|| {
            Problem::new("invalid", "asset_id is required")
                .field("asset_id")
                .hint("photos_find returns asset_id for each photo it finds.")
        })?;
        self.require("photos:read", "showing a photo")?;
        let details = self.photo_details(id).await?;
        let shown = self
            .photo_transfer(id, json!({"rendition": "display"}), SHOW_MAX_BYTES)
            .await?;
        let mut value = json!({
            "asset_id": id,
            "type": details["type"],
            "taken_at": details["created_at"],
            "width": details["width"],
            "height": details["height"],
            "image": "attached: the photo itself, full frame, nothing drawn on it",
            "next": "Show this image to the person as it is. To send it, use messages_send with photos. The image is a JPEG of the photo as Photos shows it; save gives the original file.",
        });
        if let Some(local) = crate::mcp_facade::local_time_of(details["created_at"].as_str()) {
            value["taken_at_local"] = json!(local);
        }
        if input.get("save").and_then(Value::as_bool) == Some(true) {
            let original = self.photo_original(id, &details, ATTACH_MAX_BYTES).await?;
            value["file"] = json!(self.save_photo(&original).await?);
            value["file_name"] = json!(original.name);
            value["file_bytes"] = json!(original.bytes.len());
        }
        Ok(Outcome {
            value,
            images: vec![crate::mcp::Image {
                data: STANDARD.encode(&shown),
                mime: "image/jpeg".into(),
            }],
            ..Outcome::default()
        }
        .named("photos.fetch"))
    }

    pub(crate) async fn photo_details(&self, id: &str) -> Step<Value> {
        let operation = self.operation("photos.get")?;
        let value = self
            .invoke(operation, &json!({"args": {"asset_id": id}}))
            .await
            .map_err(|problem| problem.field("asset_id"))?;
        Ok(value["result_json"].clone())
    }

    pub(crate) async fn photo_original(
        &self,
        id: &str,
        details: &Value,
        limit: u64,
    ) -> Step<PhotoFile> {
        self.photo_file(id, details, limit, false).await
    }

    async fn photo_file(
        &self,
        id: &str,
        details: &Value,
        limit: u64,
        current: bool,
    ) -> Step<PhotoFile> {
        let resources: Vec<&Value> = details["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let pick = |kinds: &[i64]| {
            kinds
                .iter()
                .find_map(|kind| resources.iter().find(|r| r["type"].as_i64() == Some(*kind)))
                .copied()
        };
        let video = details["type"] == "video";
        let chosen = match (video, current) {
            (true, true) => pick(&[CURRENT_VIDEO, ORIGINAL_VIDEO]),
            (true, false) => pick(&[ORIGINAL_VIDEO]),
            (false, true) => pick(&[CURRENT_PHOTO, ORIGINAL_PHOTO]),
            (false, false) => pick(&[ORIGINAL_PHOTO]),
        }
        .ok_or_else(|| {
            Problem::new("unsupported", "this asset has no photo or video file").field("asset_id")
        })?;
        let original = if video {
            pick(&[ORIGINAL_VIDEO])
        } else {
            pick(&[ORIGINAL_PHOTO])
        }
        .unwrap_or(chosen);
        let mut start = json!({"resource_index": chosen["index"], "max_bytes": limit});
        if let Some(key) = chosen["resource_key"].as_str() {
            start["resource_key"] = json!(key);
        }
        let bytes = self.photo_transfer(id, start, limit).await?;
        Ok(PhotoFile {
            bytes,
            name: file_name(
                original["filename"].as_str(),
                chosen["filename"].as_str(),
                id,
            ),
        })
    }

    pub(crate) async fn photo_transfer(
        &self,
        id: &str,
        mut start: Value,
        limit: u64,
    ) -> Step<Vec<u8>> {
        let operation = self.operation("photos.fetch")?;
        start["asset_id"] = json!(id);
        start["network_access"] = json!(true);
        if start.get("max_bytes").is_none() {
            start["max_bytes"] = json!(limit);
        }
        let started = self
            .invoke(operation, &json!({"args": start}))
            .await
            .map_err(|problem| problem.field("asset_id"))?;
        let transfer = started["result_json"]["transfer_id"]
            .as_str()
            .ok_or_else(|| Problem::new("corrupt", "Photos returned no transfer"))?
            .to_owned();
        let result = self.read_transfer(&transfer, limit).await;
        let release = self.operation("photos.release")?;
        let _ = self
            .invoke(release, &json!({"args": {"transfer_id": transfer}}))
            .await;
        result
    }

    async fn read_transfer(&self, transfer: &str, limit: u64) -> Step<Vec<u8>> {
        let operation = self.operation("photos.fetch")?;
        let begun = Instant::now();
        let mut bytes = Vec::new();
        loop {
            let read = match self
                .invoke(
                    operation,
                    &json!({"args": {"transfer_id": transfer, "offset": bytes.len(), "length": CHUNK}}),
                )
                .await
            {
                Err(problem) if problem.code == "capacity" && begun.elapsed() <= WAIT => {
                    tokio::time::sleep(Duration::from_millis(1100)).await;
                    continue;
                }
                read => read?,
            };
            let result = &read["result_json"];
            if result["state"] != "ready" {
                if begun.elapsed() > WAIT {
                    return Err(Problem::new(
                        "unavailable",
                        "Photos is still downloading this photo from iCloud",
                    )
                    .hint("Try again in a minute; the Mac is fetching the full photo."));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            let size = result["size"].as_u64().unwrap_or_default();
            if size > limit {
                return Err(Problem::new(
                    "capacity",
                    "this photo is too large to send this way",
                ));
            }
            let chunk = STANDARD
                .decode(result["bytes"].as_str().unwrap_or_default())
                .map_err(|_| Problem::new("corrupt", "Photos returned unreadable bytes"))?;
            if chunk.is_empty() && !result["eof"].as_bool().unwrap_or(false) {
                return Err(Problem::new("corrupt", "Photos returned an empty chunk"));
            }
            bytes.extend_from_slice(&chunk);
            if result["eof"].as_bool().unwrap_or(false) || bytes.len() as u64 >= size {
                let digest = ab_trust::fingerprint(&bytes);
                if bytes.len() as u64 != size || result["digest"].as_str() != Some(digest.as_str())
                {
                    return Err(Problem::new(
                        "corrupt",
                        "the photo changed while it was read",
                    ));
                }
                return Ok(bytes);
            }
        }
    }

    pub(crate) async fn save_photo(&self, file: &PhotoFile) -> Step<String> {
        match self
            .frontdoor
            .service
            .save_photo_file(self.principal, &file.name, &file.bytes)
            .await
        {
            Ok(path) => Ok(path),
            Err(error) => Err(from_error(
                &error,
                self.registry.find("photos.fetch"),
                self.principal,
            )),
        }
    }

    pub(crate) async fn photo_attachments(&self, ids: &[Value]) -> Step<Vec<Value>> {
        if ids.is_empty() || ids.len() > ATTACH_MAX_PHOTOS {
            return Err(Problem::new("invalid", "photos takes 1 to 10 asset IDs")
                .field("photos")
                .expected(json!(["an asset_id from photos_find"])));
        }
        self.require("photos:read", "attaching a photo")?;
        let mut files = Vec::new();
        let mut names = Vec::new();
        for id in ids {
            let id = id.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
                Problem::new("invalid", "each photo is an asset_id string").field("photos")
            })?;
            let details = self
                .photo_details(id)
                .await
                .map_err(|p| p.field("photos"))?;
            let file = self
                .photo_file(id, &details, ATTACH_MAX_BYTES, true)
                .await
                .map_err(|p| p.field("photos"))?;
            let upload = self
                .frontdoor
                .service
                .upload_message(self.principal, &file.bytes)
                .await
                .map_err(|error| {
                    from_error(&error, self.registry.find("messages.send"), self.principal)
                })?;
            let upload: Value = serde_json::from_str(upload.get()).unwrap_or(Value::Null);
            let mut name = file.name.clone();
            let mut copy = 2;
            while names.contains(&name) {
                name = numbered(&file.name, copy);
                copy += 1;
            }
            names.push(name.clone());
            files.push(json!({"name": name, "upload_id": upload["upload_id"]}));
        }
        Ok(files)
    }
}

fn file_name(original: Option<&str>, chosen: Option<&str>, id: &str) -> String {
    let safe = |name: &str| {
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && *c != '/' && *c != '\\' && *c != ':')
            .collect();
        let name = name.trim().trim_start_matches('.').to_owned();
        (!name.is_empty() && name.len() <= 200).then_some(name)
    };
    let stem = original
        .and_then(safe)
        .map(|n| n.rsplit_once('.').map_or(n.clone(), |(s, _)| s.to_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Photo-{}", &ab_trust::fingerprint(id.as_bytes())[..8]));
    let extension = chosen
        .and_then(safe)
        .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_owned()))
        .filter(|e| !e.is_empty() && e.len() <= 8)
        .unwrap_or_else(|| "jpg".into());
    format!("{stem}.{extension}")
}

fn numbered(name: &str, copy: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem} {copy}.{extension}"),
        None => format!("{name} {copy}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn edited_photos_keep_the_original_name_with_the_rendered_type() {
        assert_eq!(
            super::file_name(Some("IMG_4821.HEIC"), Some("FullSizeRender.jpeg"), "x"),
            "IMG_4821.jpeg"
        );
        let hostile = super::file_name(Some("../../.ssh/x.png"), Some("a/b"), "x");
        assert!(!hostile.contains('/') && !hostile.starts_with('.'));
        assert_eq!(super::numbered("IMG_1.HEIC", 2), "IMG_1 2.HEIC");
    }
}
