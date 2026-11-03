use crate::rawhttp::request;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/golden/oauth.json"
);
const PATHS: [&str; 3] = [
    "/.well-known/oauth-protected-resource",
    "/.well-known/oauth-protected-resource/mcp",
    "/.well-known/oauth-authorization-server",
];

fn frozen_sources() -> Result<BTreeMap<String, bool>> {
    let corpus: Value = serde_json::from_slice(
        &fs::read(GOLDEN).or_else(|_| fs::read("contracts/golden/oauth.json"))?,
    )?;
    let mut groups: BTreeMap<String, bool> = BTreeMap::new();
    groups.insert("system".into(), false);
    groups.insert("labdummy".into(), true);
    for exchange in corpus["exchanges"].as_array().into_iter().flatten() {
        let Some(metadata) = exchange["response"]
            .as_str()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        else {
            continue;
        };
        for scope in metadata["scopes_supported"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some((source, suffix)) = scope.as_str().and_then(|s| s.split_once(':'))
                && matches!(suffix, "read" | "write")
            {
                groups.insert(source.to_owned(), true);
            }
        }
    }
    if groups.len() < 3 {
        return Err(Error::Invalid("frozen OAuth source grants missing".into()));
    }
    Ok(groups)
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let groups = frozen_sources();
    evidence.check("frozen_source_grants", groups.is_ok(), "");
    let Ok(groups) = groups else {
        return Ok(());
    };
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for (source, agent) in &groups {
        let scope = format!("{source}:read,{source}:write");
        let created = world.gateway_cli(&["token", "create", "--scope", &scope]);
        evidence.check(&format!("registered_grants_{source}"), created.is_ok(), "");
        if *agent {
            expected.insert(format!("{source}:read"));
            expected.insert(format!("{source}:write"));
        }
    }
    evidence.check(
        "new_source_registered",
        expected.contains("labdummy:read") && expected.contains("labdummy:write"),
        "",
    );
    let address = world.runtime["api_address"]
        .as_str()
        .ok_or_else(|| Error::Invalid("runtime API address".into()))?
        .to_owned();
    let mut first: Option<BTreeSet<String>> = None;
    for path in PATHS {
        let reply = request(
            &address,
            "GET",
            path,
            &[("Accept", "application/json, text/event-stream")],
            None,
            10,
        )
        .ok();
        let scopes: BTreeSet<String> = reply
            .as_ref()
            .map(crate::rawhttp::Response::json)
            .and_then(|v| {
                v["scopes_supported"].as_array().map(|all| {
                    all.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
            })
            .unwrap_or_default();
        let listed = reply.as_ref().is_some_and(|r| {
            r.status == 200
                && r.json()["scopes_supported"]
                    .as_array()
                    .is_some_and(|all| !all.is_empty())
        });
        evidence.check(&format!("metadata_document_{path}"), listed, "");
        evidence.check(
            &format!("all_registered_grants_{path}"),
            scopes == expected,
            "",
        );
        match &first {
            None => first = Some(scopes),
            Some(base) => evidence.check(&format!("equal_scope_sets_{path}"), *base == scopes, ""),
        }
    }
    Ok(())
}
