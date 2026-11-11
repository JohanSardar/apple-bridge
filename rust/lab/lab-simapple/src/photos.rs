use super::{Result, Shared, Value, id, json, lock, string};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const ORIGINAL: &[u8] = b"synthetic-photo-original";
const PREVIEW: &str = "/9j/4AAQSkZJRgABAQAASABIAAD/4QBMRXhpZgAATU0AKgAAAAgAAYdpAAQAAAABAAAAGgAAAAAAA6ABAAMAAAABAAEAAKACAAQAAAABAAAABKADAAQAAAABAAAAAwAAAAD/7QA4UGhvdG9zaG9wIDMuMAA4QklNBAQAAAAAAAA4QklNBCUAAAAAABDUHYzZjwCyBOmACZjs+EJ+/8AAEQgAAwAEAwEiAAIRAQMRAf/EAB8AAAEFAQEBAQEBAAAAAAAAAAABAgMEBQYHCAkKC//EALUQAAIBAwMCBAMFBQQEAAABfQECAwAEEQUSITFBBhNRYQcicRQygZGhCCNCscEVUtHwJDNicoIJChYXGBkaJSYnKCkqNDU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6g4SFhoeIiYqSk5SVlpeYmZqio6Slpqeoqaqys7S1tre4ubrCw8TFxsfIycrS09TV1tfY2drh4uPk5ebn6Onq8fLz9PX29/j5+v/EAB8BAAMBAQEBAQEBAQEAAAAAAAABAgMEBQYHCAkKC//EALURAAIBAgQEAwQHBQQEAAECdwABAgMRBAUhMQYSQVEHYXETIjKBCBRCkaGxwQkjM1LwFWJy0QoWJDThJfEXGBkaJicoKSo1Njc4OTpDREVGR0hJSlNUVVZXWFlaY2RlZmdoaWpzdHV2d3h5eoKDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uLj5OXm5+jp6vLz9PX29/j5+v/bAEMABgYGBgYGCgYGCg4KCgoOEg4ODg4SFxISEhISFxwXFxcXFxccHBwcHBwcHCIiIiIiIicnJycnLCwsLCwsLCwsLP/bAEMBBwcHCwoLEwoKEy4fGh8uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLi4uLv/dAAQAAf/aAAwDAQACEQMRAD8A8+ooorwD6g//2Q==";

#[derive(Debug, Default)]
pub(super) struct Library {
    favorite: bool,
    albums: BTreeMap<String, (String, BTreeSet<String>)>,
    transfers: BTreeMap<String, (String, bool)>,
}

impl Library {
    pub(super) fn query(&mut self, state: &Shared, frame: &Value) -> Result<Option<Value>> {
        let operation = string(frame, "op");
        if !operation.starts_with("photos.") {
            return Ok(None);
        }
        let reply = if lock(state)?.denied.get("photos") == Some(&true) {
            Err("forbidden")
        } else {
            self.run(&operation, &frame["args"])
        };
        Ok(Some(match reply {
            Ok(result) => {
                json!({"t":"QueryResult","id":frame["id"],"result_json":result,"blobs":[]})
            }
            Err(code) => json!({"t":"QueryError","id":frame["id"],"code":code}),
        }))
    }

    fn assets(&self) -> Vec<Value> {
        vec![
            json!({"id":"photo-fixture","type":"image","created_at":"2020-01-02T00:00:00Z","favorite":self.favorite,"subtypes":0}),
            json!({"id":"video-fixture","type":"video","created_at":"2020-01-01T00:00:00Z","favorite":false,"subtypes":0}),
            json!({"id":"live-fixture","type":"image","created_at":"2019-01-01T00:00:00Z","favorite":false,"subtypes":8}),
        ]
    }

    fn run(&mut self, operation: &str, args: &Value) -> std::result::Result<Value, &'static str> {
        let principal = args["_principal"].as_str().ok_or("forbidden")?;
        match operation {
            "photos.search" => {
                let mut assets = self.assets();
                assets.retain(|asset| {
                    args["type"].as_str().is_none_or(|kind| {
                        if kind == "live_photo" {
                            asset["id"] == "live-fixture"
                        } else {
                            asset["type"] == kind
                        }
                    }) && args["favorite"]
                        .as_bool()
                        .is_none_or(|favorite| asset["favorite"] == favorite)
                        && args["start"].as_str().is_none_or(|start| {
                            asset["created_at"]
                                .as_str()
                                .is_some_and(|time| time >= start)
                        })
                        && args["end"].as_str().is_none_or(|end| {
                            asset["created_at"].as_str().is_some_and(|time| time < end)
                        })
                        && args["album_id"].as_str().is_none_or(|album| {
                            self.albums.get(album).is_some_and(|(_, members)| {
                                members.contains(asset["id"].as_str().unwrap_or_default())
                            })
                        })
                });
                if let Some(cursor) = args["cursor"].as_str() {
                    let cursor: Value =
                        serde_json::from_slice(&STANDARD.decode(cursor).map_err(|_| "invalid")?)
                            .map_err(|_| "invalid")?;
                    let date = cursor["created_at"].as_str().ok_or("invalid")?;
                    let id = cursor["id"].as_str().ok_or("invalid")?;
                    assets.retain(|asset| {
                        let created = asset["created_at"].as_str().unwrap_or_default();
                        created < date
                            || created == date && asset["id"].as_str().unwrap_or_default() > id
                    });
                }
                let limit =
                    usize::try_from(args["limit"].as_u64().unwrap_or(50)).map_err(|_| "invalid")?;
                let near = args.get("near").is_some();
                let end = assets.len().min(if near { 1000 } else { limit });
                let next = if end < assets.len() {
                    let last = &assets[end - 1];
                    json!(
                        STANDARD.encode(
                            serde_json::to_vec(
                                &json!({"created_at":last["created_at"],"id":last["id"]}),
                            )
                            .map_err(|_| "invalid")?,
                        )
                    )
                } else {
                    Value::Null
                };
                let rows = if near { &assets[..0] } else { &assets[..end] };
                Ok(json!({"assets":rows,"examined_count":end,"next_cursor":next,"limited":false}))
            }
            "photos.get" => {
                let mut asset = self
                    .assets()
                    .into_iter()
                    .find(|asset| asset["id"] == args["asset_id"])
                    .ok_or("invalid")?;
                let mut resources = if args["asset_id"] == "live-fixture" {
                    vec![
                        json!({"index":0,"type":1,"filename":"IMG_0003.HEIC","uti":"public.heic"}),
                        json!({"index":1,"type":9,"filename":"IMG_0003.MOV","uti":"com.apple.quicktime-movie"}),
                    ]
                } else if args["asset_id"] == "video-fixture" {
                    vec![
                        json!({"index":0,"type":2,"filename":"IMG_0002.MOV","uti":"com.apple.quicktime-movie"}),
                    ]
                } else {
                    vec![json!({"index":0,"type":1,"filename":"IMG_0001.HEIC","uti":"public.heic"})]
                };
                for resource in &mut resources {
                    resource["resource_key"] = json!("0".repeat(64));
                }
                asset["resources"] = json!(resources);
                Ok(asset)
            }
            "photos.preview" => {
                let asset = self
                    .assets()
                    .into_iter()
                    .find(|asset| asset["id"] == args["asset_id"])
                    .ok_or("invalid")?;
                Ok(
                    json!({"asset_id":asset["id"],"type":asset["type"],"created_at":asset["created_at"],"mime_type":"image/jpeg","width":4,"height":3,"bytes":STANDARD.decode(PREVIEW).map_err(|_| "corrupt")?.len(),"image_base64":PREVIEW}),
                )
            }
            "photos.contact_sheet" => {
                let assets = self.assets();
                let mut cells = Vec::new();
                for (index, id) in args["asset_ids"]
                    .as_array()
                    .ok_or("invalid")?
                    .iter()
                    .enumerate()
                {
                    let asset = assets
                        .iter()
                        .find(|asset| &asset["id"] == id)
                        .ok_or("invalid")?;
                    cells.push(json!({"number":index + 1,"asset_id":id,"type":asset["type"],"created_at":asset["created_at"],"rendered":true}));
                }
                Ok(
                    json!({"cells":cells,"columns":cells.len().min(5),"mime_type":"image/jpeg","width":4,"height":3,"bytes":STANDARD.decode(PREVIEW).map_err(|_| "corrupt")?.len(),"image_base64":PREVIEW}),
                )
            }
            "photos.albums" => Ok(
                json!({"albums":self.albums.iter().map(|(id,(title,_))| json!({"id":id,"title":title})).collect::<Vec<_>>(),"count":self.albums.len(),"next_offset":null}),
            ),
            "photos.fetch" => {
                if let Some(transfer) = args["transfer_id"].as_str() {
                    let Some((owner, display)) = self.transfers.get(transfer) else {
                        return Err("expired");
                    };
                    if owner != principal {
                        return Err("expired");
                    }
                    let content = if *display {
                        STANDARD.decode(PREVIEW).map_err(|_| "corrupt")?
                    } else {
                        ORIGINAL.to_vec()
                    };
                    let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0))
                        .map_err(|_| "invalid")?;
                    let length = usize::try_from(args["length"].as_u64().unwrap_or(128 << 10))
                        .map_err(|_| "invalid")?;
                    if offset > content.len() {
                        return Err("invalid");
                    }
                    let end = (offset + length.min(64 << 10)).min(content.len());
                    return Ok(
                        json!({"transfer_id":transfer,"state":"ready","digest":format!("{:x}",Sha256::digest(&content)),"size":content.len(),"offset":offset,"bytes":STANDARD.encode(&content[offset..end]),"eof":end==content.len()}),
                    );
                }
                if !self
                    .assets()
                    .iter()
                    .any(|asset| asset["id"] == args["asset_id"])
                {
                    return Err("invalid");
                }
                if args["resource_key"]
                    .as_str()
                    .is_some_and(|key| key != "0".repeat(64))
                {
                    return Err("conflict");
                }
                let display = match args["rendition"].as_str() {
                    None => false,
                    Some("display") => true,
                    Some(_) => return Err("invalid"),
                };
                let transfer = id("photo").map_err(|_| "unavailable")?;
                self.transfers
                    .insert(transfer.clone(), (principal.into(), display));
                Ok(
                    json!({"transfer_id":transfer,"state":"downloading","idle_expiry_s":120,"max_lifetime_s":86400}),
                )
            }
            "photos.release" => {
                let transfer = args["transfer_id"].as_str().ok_or("invalid")?;
                if self
                    .transfers
                    .get(transfer)
                    .map(|(owner, _)| owner.as_str())
                    != Some(principal)
                {
                    return Err("expired");
                }
                self.transfers.remove(transfer);
                Ok(json!({"released":true}))
            }
            "photos.favorite" => {
                if args["asset_id"] != "photo-fixture" {
                    return Err("invalid");
                }
                self.favorite = args["favorite"].as_bool().ok_or("invalid")?;
                Ok(json!({"completed":true}))
            }
            "photos.album_create" => {
                let title = args["title"].as_str().ok_or("invalid")?;
                let album = id("album").map_err(|_| "unavailable")?;
                self.albums
                    .insert(album.clone(), (title.into(), BTreeSet::new()));
                Ok(json!({"completed":true,"album_id":album}))
            }
            "photos.album_rename"
            | "photos.album_add"
            | "photos.album_remove"
            | "photos.album_delete" => {
                let id = args["album_id"].as_str().ok_or("invalid")?;
                if operation == "photos.album_delete" {
                    self.albums.remove(id).ok_or("invalid")?;
                } else {
                    let (title, members) = self.albums.get_mut(id).ok_or("invalid")?;
                    if operation == "photos.album_rename" {
                        *title = args["title"].as_str().ok_or("invalid")?.into();
                    } else {
                        for asset in args["asset_ids"].as_array().ok_or("invalid")? {
                            let asset = asset.as_str().ok_or("invalid")?;
                            if operation == "photos.album_add" {
                                members.insert(asset.into());
                            } else {
                                members.remove(asset);
                            }
                        }
                    }
                }
                Ok(json!({"completed":true}))
            }
            _ => Err("unsupported"),
        }
    }
}
