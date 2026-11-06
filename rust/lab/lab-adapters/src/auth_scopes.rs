use crate::rawhttp::request;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
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
const AUTHORIZATION_SERVER: &str = "/.well-known/oauth-authorization-server";
const PATHS: [&str; 3] = [
    "/.well-known/oauth-protected-resource",
    "/.well-known/oauth-protected-resource/mcp",
    AUTHORIZATION_SERVER,
];
const OFFLINE_ACCESS: &str = "offline_access";

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

fn offline_access_grants_nothing(world: &World, address: &str, evidence: &mut Evidence) {
    evidence.check(
        "offline_access_is_not_a_token_grant",
        world
            .gateway_cli(&["token", "create", "--scope", OFFLINE_ACCESS])
            .is_err(),
        "",
    );
    let registration = request(
        address,
        "POST",
        "/oauth/register",
        &[("Content-Type", "application/json")],
        Some(br#"{"client_name":"Bridge Test scopes","redirect_uris":["https://connector.example/callback"],"token_endpoint_auth_method":"none"}"#),
        10,
    );
    let client = registration
        .as_ref()
        .map(|reply| {
            reply.json()["client_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap_or_default();
    let challenge = URL_SAFE_NO_PAD.encode([7u8; 32]);
    let refused = request(
        address,
        "GET",
        &format!(
            "/oauth/authorize?client_id={client}&redirect_uri=https%3A%2F%2Fconnector.example%2Fcallback&response_type=code&code_challenge_method=S256&code_challenge={challenge}&scope={OFFLINE_ACCESS}"
        ),
        &[],
        None,
        10,
    );
    evidence.check(
        "offline_access_alone_is_an_invalid_scope",
        !client.is_empty()
            && refused.is_ok_and(|reply| {
                reply.status == 303
                    && reply
                        .header("Location")
                        .is_some_and(|to| to.contains("error=invalid_scope"))
            }),
        "",
    );
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
        let mut scopes: BTreeSet<String> = reply
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
        if path == AUTHORIZATION_SERVER {
            evidence.check(
                "authorization_server_lists_offline_access",
                scopes.remove(OFFLINE_ACCESS),
                "",
            );
        }
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
    offline_access_grants_nothing(world, &address, evidence);
    Ok(())
}
