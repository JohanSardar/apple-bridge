use crate::http::{Client, random_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn call(client: &Client, operation: &str, args: Value, mcp: bool) -> Result<Value> {
    let mut request = json!({"id":random_id("qry")?,"deadline_ms":5000});
    request["args"] = args;
    let response = if mcp {
        client.call_mcp_raw(operation, &request, 10)?
    } else {
        client.call_raw(operation, &request, 10)?
    };
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
