use crate::{
    assertions::await_until,
    http::{Client, random_id},
    replicas,
};
use base64::Engine;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const REAL: [&str; 1] = ["FILE-REPLICA"];
const SCOPE: &str = "files:read,files:write,system:read";
const CONTENT: &[u8] = b"Bridge Test synthetic replica payload";
const PRESERVED: &[u8] = b"Bridge Test preserve";

fn cleanup(folder: &Path, name: &str) -> Result<()> {
    let meta = match fs::symlink_metadata(folder) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let parent_ok = folder
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "Bridge Test");
    let owned = folder
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("[ab-test "));
    if !meta.is_dir() || !parent_ok || !owned {
        return Err(Error::Invalid("unowned fixture".into()));
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let file = entry.file_name().to_string_lossy().into_owned();
        let regular = entry.file_type()?.is_file();
        let data = fs::read(entry.path())?;
        if (file != name && file != "created.txt") || !regular || data != CONTENT {
            return Err(Error::Invalid("unowned fixture".into()));
        }
    }
    fs::remove_dir_all(folder)?;
    Ok(())
}

fn query(client: &Client, op: &str, args: &Value, deadline: i64) -> Result<Option<Value>> {
    let body = json!({"id": random_id("qry")?, "args": args, "deadline_ms": deadline});
    client.call(op, &body, 40)
}

#[allow(clippy::too_many_lines)]
fn flow(
    world: &World,
    evidence: &mut Evidence,
    root: &Path,
    folder: &Path,
    name: &str,
) -> Result<()> {
    let started = replicas::start_with(world, 1, SCOPE, 0);
    evidence.check("ubuntu_replica_started", started.is_ok(), "");
    if let Err(error) = started {
        eprintln!("{error}");
        return Ok(());
    }
    let endpoints = replicas::endpoints();
    let Some(endpoint) = endpoints.first() else {
        return Ok(());
    };
    let client = endpoint.client();
    let target = folder.join(name).to_string_lossy().into_owned();
    let found = await_until(30, || {
        query(
            &client,
            "files.search",
            &json!({"query": name, "field": "name"}),
            10_000,
        )
        .ok()
        .flatten()
        .is_some_and(|reply| {
            reply["result_json"]["items"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["path"] == json!(target)))
        })
    });
    evidence.check("ubuntu_search", found, "");
    if !found {
        return Ok(());
    }
    let relative = Path::new(folder.file_name().unwrap_or_default()).join(name);
    let read = query(
        &client,
        "files.read",
        &json!({"root": root.to_string_lossy(), "path": relative.to_string_lossy()}),
        15_000,
    )?;
    let blobs = read
        .as_ref()
        .and_then(|reply| reply["blobs"].as_array().cloned())
        .unwrap_or_default();
    evidence.check("ubuntu_read_blob_reference", blobs.len() == 1, "");
    if blobs.len() != 1 {
        return Ok(());
    }
    let digest = blobs[0]["digest"].as_str().unwrap_or_default().to_owned();
    evidence.check("blob_request", true, "");
    let record = random_id("qry")?;
    let fetched = client.request(
        "GET",
        &format!("/v1/blobs/{digest}?record={record}"),
        &[],
        None,
        30,
    );
    evidence.check("ubuntu_blob_http", fetched.is_ok(), "");
    let Ok(fetched) = fetched else {
        return Ok(());
    };
    let valid = fetched.status == 200
        && hex::encode(Sha256::digest(&fetched.body)) == digest
        && fetched.body == CONTENT;
    evidence.check("ubuntu_blob_digest", valid, "");
    if !valid {
        return Ok(());
    }
    let create = json!({
        "command_id": random_id("cmd")?, "op": "files.create", "wait_ms": 15000,
        "args": {"root": root.to_string_lossy(), "parent": folder.file_name().unwrap_or_default().to_string_lossy(),
            "name": "created.txt",
            "content_base64": base64::engine::general_purpose::STANDARD.encode(CONTENT),
            "digest": hex::encode(Sha256::digest(CONTENT))}
    });
    let command = client.call("files.create", &create, 40)?;
    let observed = command.is_some_and(|value| value["state"] == "observed");
    evidence.check("ubuntu_create_observed", observed, "");
    if !observed {
        return Ok(());
    }
    evidence.check(
        "filesystem_truth_digest",
        fs::read(folder.join("created.txt")).is_ok_and(|data| data == CONTENT),
        "",
    );
    let canary = folder.join("unowned.txt");
    let wrote = fs::write(&canary, PRESERVED).is_ok();
    evidence.check("cleanup_canary_created", wrote, "");
    if !wrote {
        return Ok(());
    }
    let refused = cleanup(folder, name).is_err();
    let kept = fs::read(&canary).is_ok_and(|data| data == PRESERVED);
    evidence.check("unowned_fixture_preserved", refused && kept, "");
    if !(refused && kept) {
        return Ok(());
    }
    let removed = fs::remove_file(&canary).is_ok();
    evidence.check("cleanup_canary_removed", removed, "");
    if !removed {
        return Ok(());
    }
    evidence.check("owned_fixture_cleanup", cleanup(folder, name).is_ok(), "");
    Ok(())
}

pub fn real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let home = env::var("HOME");
    evidence.check("home", home.is_ok(), "");
    let home = home.map_err(|_| Error::Invalid("HOME".into()))?;
    let root: PathBuf =
        Path::new(&home).join("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test");
    let run = random_id("run")?;
    let folder = root.join(format!("[ab-test {run}]"));
    let prepared = fs::create_dir_all(&folder)
        .and_then(|()| fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)))
        .is_ok();
    evidence.check("fixture_directory", prepared, "");
    if !prepared {
        return Ok(());
    }
    let name = format!("abtest{}.txt", run.replace('_', ""));
    let written = fs::write(folder.join(&name), CONTENT).is_ok();
    evidence.check("fixture_write", written, "");
    let result = if written {
        flow(world, evidence, &root, &folder, &name)
    } else {
        Ok(())
    };
    let _ = cleanup(&folder, &name);
    result
}
