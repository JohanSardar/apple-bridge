use crate::http::{Client, random_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn call(client: &Client, operation: &str, args: Value, _mcp: bool) -> Result<Value> {
    let mut request = json!({"id":random_id("qry")?,"deadline_ms":5000});
    request["args"] = args;
    let response = client.call_raw(operation, &request, 10)?;
    let (status, result) =
        response.ok_or_else(|| Error::Invalid(format!("{operation}: missing query response")))?;
    if status != 200 || !result["error"].is_null() || result["result_json"].is_null() {
        return Err(Error::Invalid(format!(
            "{operation}: HTTP {status}, code {}, state {}",
            result["error"]["code"].as_str().unwrap_or_default(),
            result["state"].as_str().unwrap_or_default()
        )));
    }
    Ok(result["result_json"].clone())
}

fn cli(world: &World, client: &Client, operation: &str, args: Value, write: bool) -> Result<Value> {
    let mut request = json!({"id":random_id("qry")?,"deadline_ms":5000});
    request["args"] = args;
    let body = request.to_string();
    let mut arguments = vec![
        "call",
        operation,
        "--token-env",
        "AB_PHOTOS_PROOF_TOKEN",
        "--args",
        &body,
    ];
    if write {
        arguments.push("--write");
    }
    let response: Value = serde_json::from_str(
        &world.gateway_cli_env(&arguments, &[("AB_PHOTOS_PROOF_TOKEN", client.token())])?,
    )?;
    if !response["error"].is_null() || response["result_json"].is_null() {
        return Err(Error::Invalid("Photos CLI query failed".into()));
    }
    Ok(response["result_json"].clone())
}

pub fn sim(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "photos:read,photos:write,system:read")?;
    let ready = crate::assertions::await_until(20, || {
        call(&client, "helper.version", json!({}), false).is_ok()
    });
    evidence.check("helper_ready", ready, "");
    let terminal: Value =
        serde_json::from_str(&world.gateway_cli(&["read", "photos", "--limit", "1", "--json"])?)?;
    evidence.check(
        "cli_live_read",
        terminal["result_json"]["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "",
    );
    queries(&client, evidence)?;
    previews(&client, evidence)?;
    shown(world, evidence)?;
    originals(world, &client, evidence)?;
    albums(world, &client, evidence)?;
    denial(world, &client, evidence)
}

fn queries(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let search = call(client, "photos.search", json!({"limit":1}), false)?;
    evidence.check(
        "bounded_first_page",
        search["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1)
            && search["next_cursor"].is_string()
            && search["examined_count"] == 1
            && search.get("candidate_count").is_none(),
        "",
    );
    let next = call(
        client,
        "photos.search",
        json!({"limit":1,"cursor":search["next_cursor"]}),
        true,
    )?;
    evidence.check(
        "next_page_advances",
        next["assets"][0]["id"] != search["assets"][0]["id"]
            && next["next_cursor"].is_string()
            && next["examined_count"] == 1,
        "",
    );
    let last = call(
        client,
        "photos.search",
        json!({"limit":1,"cursor":next["next_cursor"]}),
        false,
    )?;
    evidence.check(
        "cursor_enumeration_ends_without_full_count",
        last["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1)
            && last["assets"][0]["id"] != next["assets"][0]["id"]
            && last["assets"][0]["id"] != search["assets"][0]["id"]
            && last["next_cursor"].is_null()
            && last["examined_count"] == 1,
        "",
    );
    let search = call(client, "photos.search", json!({"type":"video"}), true)?;
    evidence.check(
        "mcp_video_filter",
        search["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1 && rows[0]["type"] == "video"),
        "",
    );
    let search = call(
        client,
        "photos.search",
        json!({"start":"2020-01-01T00:00:00Z","end":"2020-01-02T00:00:00Z"}),
        false,
    )?;
    evidence.check(
        "half_open_dates",
        search["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "",
    );
    let details = call(
        client,
        "photos.get",
        json!({"asset_id":"live-fixture"}),
        true,
    )?;
    evidence.check(
        "live_photo_resource_pair",
        details["resources"]
            .as_array()
            .is_some_and(|rows| rows.len() == 2 && rows[1]["type"] == 9),
        "",
    );
    Ok(())
}

fn previews(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let preview = client.call_mcp_result(
        "apple_read",
        &json!({"operation":"photos.preview","arguments":{"asset_id":"photo-fixture","max_px":256},"verbose":true}),
        10,
    )?;
    let image = &preview["content"][1];
    evidence.check(
        "mcp_preview_is_image_content",
        image["type"] == "image"
            && image["mimeType"] == "image/jpeg"
            && image["data"]
                .as_str()
                .and_then(|data| STANDARD.decode(data).ok())
                .is_some_and(|bytes| bytes.starts_with(&[0xff, 0xd8]))
            && preview["structuredContent"]["result_json"]
                .get("image_base64")
                .is_none()
            && !preview["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .contains("image_base64"),
        "",
    );
    let rest = call(
        client,
        "photos.preview",
        json!({"asset_id":"photo-fixture"}),
        false,
    )?;
    evidence.check(
        "rest_preview_keeps_base64",
        rest["image_base64"].is_string() && rest["mime_type"] == "image/jpeg",
        "",
    );
    let sheet = client.call_mcp_result(
        "apple_read",
        &json!({"operation":"photos.preview_grid","arguments":{"asset_ids":["photo-fixture","video-fixture","live-fixture"]},"verbose":true}),
        10,
    )?;
    let cells = &sheet["structuredContent"]["result_json"]["cells"];
    evidence.check(
        "contact_sheet_numbers_cells",
        sheet["content"][1]["type"] == "image"
            && cells.as_array().is_some_and(|cells| cells.len() == 3)
            && cells[2]["number"] == 3
            && cells[2]["asset_id"] == "live-fixture",
        "",
    );
    let duplicate = client.call_raw(
        "photos.contact_sheet",
        &json!({"args":{"asset_ids":["photo-fixture","photo-fixture"]}}),
        10,
    )?;
    evidence.check(
        "contact_sheet_rejects_duplicates",
        duplicate.is_some_and(|(_, value)| value["error"]["code"] == "invalid"),
        "",
    );
    Ok(())
}

fn shown(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "photos:read,messages:read,messages:write")?;
    let shown = client.call_mcp_result(
        "photos_show",
        &json!({"asset_id":"photo-fixture","save":true}),
        30,
    )?;
    let image = shown["content"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["type"] == "image"))
        .and_then(|item| item["data"].as_str())
        .and_then(|data| STANDARD.decode(data).ok())
        .unwrap_or_default();
    let file = shown["structuredContent"]["file"]
        .as_str()
        .unwrap_or_default();
    evidence.check(
        "show_returns_the_display_rendition_alone",
        shown["isError"] != true
            && image.starts_with(&[0xff, 0xd8])
            && shown["structuredContent"].get("photos").is_none(),
        &format!("isError {} image {} bytes", shown["isError"], image.len()),
    );
    evidence.check(
        "show_saves_the_original_file",
        file.ends_with("/IMG_0001.HEIC")
            && std::fs::read(file).is_ok_and(|bytes| bytes == b"synthetic-photo-original"),
        "",
    );
    let one = client.call_mcp_result("photos_find", &json!({"type":"video"}), 30)?;
    evidence.check(
        "one_match_returns_the_photo_not_a_sheet",
        one["structuredContent"]["photos"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1 && rows[0].get("number").is_none())
            && one["content"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["type"] == "image")),
        "",
    );
    sent(world, &client, evidence)
}

fn sent(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    world.control(
        "/messages",
        &json!({"action": "seed", "key": "", "count": 0}),
    )?;
    let mut chat_id = String::new();
    crate::assertions::await_until(20, || {
        chat_id = client
            .call_mcp_result(
                "apple_read",
                &json!({"operation":"bridge.list_records","arguments":{"collection":"messages.chat","limit":1}}),
                10,
            )
            .ok()
            .and_then(|r| r["structuredContent"]["records"][0]["id"].as_str().map(str::to_owned))
            .unwrap_or_default();
        !chat_id.is_empty()
    });
    let tag = random_id("photo")?;
    let request = random_id("req")?;
    let sent = client.call_mcp_result(
        "messages_send",
        &json!({"chat_id":chat_id,"text":format!("[ab-test {tag}] photo"),"photos":["photo-fixture"],"request_id":request}),
        30,
    )?;
    evidence.check(
        "send_attaches_the_original",
        sent["structuredContent"]["status"] == "done",
        &sent["structuredContent"].to_string(),
    );
    let again = client.call_mcp_result(
        "messages_send",
        &json!({"chat_id":chat_id,"text":format!("[ab-test {tag}] photo"),"photos":["photo-fixture"],"request_id":request}),
        30,
    )?;
    evidence.check(
        "repeated_request_id_does_not_send_twice",
        again["structuredContent"]["status"] == "done"
            && again["structuredContent"]["ids"] == sent["structuredContent"]["ids"],
        "",
    );
    let shared = client.call_mcp_result(
        "apple_read",
        &json!({"operation":"messages.list_shared","arguments":{"chat_id":chat_id,"kind":"image"}}),
        30,
    )?;
    let original = format!("{:x}", Sha256::digest(b"synthetic-photo-original"));
    evidence.check(
        "sent_file_is_the_untouched_original",
        shared["structuredContent"]["items"]
            .as_array()
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item["filename"] == "IMG_0001.HEIC" && item["digest"] == original.as_str()
                })
            }),
        &shared["structuredContent"].to_string(),
    );
    Ok(())
}

fn originals(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let stale = client.call_raw(
        "photos.fetch",
        &json!({"args":{"asset_id":"photo-fixture","resource_key":"1".repeat(64)}}),
        10,
    )?;
    evidence.check(
        "stale_resource_key_is_conflict",
        stale.is_some_and(|(_, value)| value["error"]["code"] == "conflict"),
        "",
    );
    let transfer = call(
        client,
        "photos.fetch",
        json!({"asset_id":"photo-fixture","network_access":true}),
        false,
    )?["transfer_id"]
        .clone();
    let other = Client::new(world, "photos:read")?;
    let foreign = other.call_raw(
        "photos.fetch",
        &json!({"args":{"transfer_id":transfer}}),
        10,
    )?;
    evidence.check(
        "transfer_principal_bound",
        foreign.is_some_and(|(_, value)| !value["error"].is_null()),
        "",
    );
    let first = cli(
        world,
        client,
        "photos.fetch",
        json!({"transfer_id":transfer,"offset":0,"length":1}),
        false,
    )?;
    evidence.check(
        "cli_transfer_preserves_identity",
        first["size"] == 24 && first["bytes"] == "cw==",
        "",
    );
    let mut bytes = Vec::new();
    loop {
        let response = call(
            client,
            "photos.fetch",
            json!({"transfer_id":transfer,"offset":bytes.len(),"length":5}),
            true,
        )?;
        let chunk = STANDARD
            .decode(response["bytes"].as_str().unwrap_or_default())
            .map_err(|_| Error::Invalid("photo chunk".into()))?;
        if chunk.is_empty() {
            return Err(Error::Invalid("empty photo chunk".into()));
        }
        bytes.extend(chunk);
        if response["eof"] == true {
            break;
        }
    }
    evidence.check(
        "original_sha256",
        hex::encode(Sha256::digest(&bytes))
            == "6227f0512550e6e852fe771528203afbb5ea78ef72d5477c5d6c105de2bbc748",
        "",
    );
    cli(
        world,
        client,
        "photos.release",
        json!({"transfer_id":transfer}),
        false,
    )?;
    let released = client.call_raw(
        "photos.fetch",
        &json!({"args":{"transfer_id":transfer}}),
        10,
    )?;
    evidence.check(
        "released_transfer_is_expired",
        released.is_some_and(|(_, value)| value["error"]["code"] == "expired"),
        "",
    );
    Ok(())
}

fn albums(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let album = cli(
        world,
        client,
        "photos.album_create",
        json!({"title":"synthetic-album"}),
        true,
    )?["album_id"]
        .clone();
    call(
        client,
        "photos.album_rename",
        json!({"album_id":album,"title":"synthetic-renamed"}),
        true,
    )?;
    let renamed = call(client, "photos.albums", json!({}), false)?;
    evidence.check(
        "album_created_and_renamed",
        renamed["albums"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| row["id"] == album && row["title"] == "synthetic-renamed")
        }),
        "",
    );
    call(
        client,
        "photos.album_add",
        json!({"album_id":album,"asset_ids":["photo-fixture"]}),
        false,
    )?;
    let members = call(client, "photos.search", json!({"album_id":album}), true)?;
    evidence.check(
        "album_membership_added",
        members["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "",
    );
    call(
        client,
        "photos.album_remove",
        json!({"album_id":album,"asset_ids":["photo-fixture"]}),
        true,
    )?;
    let members = call(client, "photos.search", json!({"album_id":album}), false)?;
    evidence.check(
        "album_membership_removed",
        members["assets"].as_array().is_some_and(Vec::is_empty),
        "",
    );
    call(
        client,
        "photos.album_delete",
        json!({"album_id":album}),
        false,
    )?;
    let albums = call(client, "photos.albums", json!({}), true)?;
    evidence.check(
        "album_deleted_without_assets",
        albums["count"] == 0
            && call(client, "photos.search", json!({}), false)?["assets"]
                .as_array()
                .is_some_and(|rows| rows.len() == 3),
        "",
    );
    favorite(world, client, evidence)
}

fn favorite(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let read_only = Client::new(world, "photos:read")?;
    let forbidden =
        json!({"id":random_id("qry")?,"args":{"asset_id":"photo-fixture","favorite":true}})
            .to_string();
    evidence.check(
        "cli_write_scope_enforced",
        world
            .gateway_cli_env(
                &[
                    "call",
                    "photos.favorite",
                    "--write",
                    "--token-env",
                    "AB_PHOTOS_PROOF_TOKEN",
                    "--args",
                    &forbidden,
                ],
                &[("AB_PHOTOS_PROOF_TOKEN", read_only.token())],
            )
            .is_err(),
        "",
    );
    cli(
        world,
        client,
        "photos.favorite",
        json!({"asset_id":"photo-fixture","favorite":true}),
        true,
    )?;
    let favorites = call(client, "photos.search", json!({"favorite":true}), false)?;
    evidence.check(
        "favorite_readback",
        favorites["assets"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "",
    );
    Ok(())
}

fn denial(world: &mut World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    world.control(
        "/source-state",
        &json!({"source":"photos","state":"permission_missing"}),
    )?;
    let denied = client.call_raw("photos.search", &json!({"args":{}}), 10)?;
    evidence.check(
        "permission_denial_has_own_hint",
        denied.is_some_and(|(status, value)| {
            status == 403
                && value["error"]["code"] == "forbidden"
                && value["error"]["message"].as_str().is_some_and(|message| {
                    message.contains("Photos") && message.contains("Other sources")
                })
        }),
        "",
    );
    evidence.check(
        "other_source_remains_available",
        call(client, "helper.version", json!({}), false).is_ok(),
        "",
    );
    Ok(())
}
