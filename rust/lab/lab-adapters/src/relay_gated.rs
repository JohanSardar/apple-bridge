use crate::relay::{Fixture, Lab, Limits, Tunnel, Wire, b64, cred, enroll_with, http, signing_key};
use lab_runner::Evidence;
use lab_world::Result;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn run(binary: &Path, directory: &Path, evidence: &mut Evidence) -> Result<()> {
    let limits = Limits::standard();
    let state = directory.join("state.json");
    let config = directory.join("owner-keys");
    let owner = signing_key()?;
    let unknown = signing_key()?;
    let public = b64(owner.verifying_key().as_bytes());
    write_private(&config, public.as_bytes())?;
    let owner_rpc = |fixture: &Fixture| {
        let _ = fixture.rpc("owner", json!({"path": config.to_string_lossy()}));
    };
    let store = Arc::new(Fixture::open(binary, &state, limits)?);
    owner_rpc(&store);
    let lab = Lab::start_with(&store, "")?;
    for endpoint in ["/enroll", "/rotate"] {
        let (_, code) = enroll_with(&lab.api, &unknown, "", endpoint);
        evidence.check(&format!("unknown_refused{endpoint}"), code == 403, "");
    }
    let (a, code) = enroll_with(&lab.api, &owner, "", "/enroll");
    evidence.check("owner_enroll", code == 200, "");
    let (b, code) = enroll_with(&lab.api, &owner, "", "/rotate");
    evidence.check(
        "owner_rotate",
        code == 200
            && cred(&a, "hostname") == cred(&b, "hostname")
            && cred(&a, "credential") != cred(&b, "credential"),
        "",
    );
    let host = cred(&a, "hostname");
    let before = store.device(&host);
    store.policy(&host, limits, true, false);
    let (disabled_credential, code) = enroll_with(&lab.api, &owner, "", "/rotate");
    let disabled = store.device(&host);
    evidence.check(
        "disabled_preserved",
        code == 200
            && disabled["disabled"].as_bool() == Some(true)
            && disabled["bytes"] == before["bytes"]
            && disabled["month"] == before["month"]
            && disabled["limits"] == before["limits"],
        "",
    );
    drop(lab);
    let store = Arc::new(Fixture::open(binary, &state, limits)?);
    owner_rpc(&store);
    let lab = Lab::start_with(&store, "")?;
    let (_, code) = enroll_with(&lab.api, &unknown, "", "/rotate");
    evidence.check("restart_gate", code == 403, "");
    evidence.check(
        "restart_disabled",
        store.device(&host)["disabled"].as_bool() == Some(true),
        "",
    );
    let refused = lab.attach_kept(
        &host,
        &cred(&disabled_credential, "credential"),
        Tunnel::Dial("127.0.0.1:1".into()),
        &Wire::new(&[]),
    );
    evidence.check("disabled_attach_refused", refused.is_err(), "");
    fs::remove_file(&config)?;
    let frozen = fs::read(&state)?;
    for endpoint in ["/enroll", "/rotate"] {
        let (_, code) = enroll_with(&lab.api, &owner, "", endpoint);
        evidence.check(&format!("missing_config_closed{endpoint}"), code == 403, "");
    }
    write_private(&config, b"invalid")?;
    let (_, code) = enroll_with(&lab.api, &owner, "", "/rotate");
    evidence.check("invalid_config_closed", code == 403, "");
    evidence.check("denial_preserves_state", frozen == fs::read(&state)?, "");
    write_private(&config, public.as_bytes())?;
    store.policy(&host, limits, false, true);
    let (_, code) = enroll_with(&lab.api, &owner, "", "/rotate");
    evidence.check("permanent_revocation", code == 403, "");
    let rollback = Fixture::open(binary, &state, limits);
    evidence.check("legacy_state_readable", rollback.is_ok(), "");
    if let Ok(rollback) = rollback {
        evidence.check(
            "rollback_revoked",
            rollback.device(&host)["revoked"].as_bool() == Some(true),
            "",
        );
    }
    drop(lab);
    let lab = Lab::start_with(&store, "")?;
    let mut status = 0;
    for _ in 0..31 {
        status = http(&lab.api, "GET", "/challenge", None, None).map_or(0, |r| r.status);
    }
    evidence.check("challenge_throttle", status == 429, "");
    let empty = json!({
        "public_key": "", "nonce": "", "signature": "", "hostname": ""
    });
    for _ in 0..21 {
        status = http(
            &lab.api,
            "POST",
            "/rotate",
            None,
            Some(empty.to_string().as_bytes()),
        )
        .map_or(0, |r| r.status);
    }
    evidence.check("shared_enrollment_throttle", status == 429, "");
    Ok(())
}
