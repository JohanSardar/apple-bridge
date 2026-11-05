#![allow(clippy::unwrap_used)]
use super::{Client, Code, Exchange, Refresh, Tokens, display_name, read, write};
use crate::{Authority, Grant, Principal};
use serde_json::Value;

const RESOURCE: &str = "https://bridge.example/mcp";
const VERIFIER: &str = "synthetic-verifier-synthetic-verifier-synthetic-verifier";
const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";

fn client(name: &str, uris: &[String]) -> Client {
    Client {
        client_name: name.into(),
        redirect_uris: uris.to_vec(),
        ..Client::default()
    }
}

fn exchange(grant_type: &str, refresh_token: &str, code: &str) -> Exchange {
    Exchange {
        grant_type: grant_type.into(),
        client_id: "client_synthetic".into(),
        code: code.into(),
        redirect_uri: CALLBACK.into(),
        code_verifier: VERIFIER.into(),
        resource: RESOURCE.into(),
        refresh_token: refresh_token.into(),
        scope: String::new(),
    }
}

fn connected(authority: &Authority) -> (Principal, Tokens) {
    let scopes = [Grant::parse("calendar:read").unwrap()];
    let (agent, _) = authority
        .create_oauth_agent("Synthetic", "approve", &scopes, false)
        .unwrap();
    let challenge = oauth2::PkceCodeChallenge::from_code_verifier_sha256(
        &oauth2::PkceCodeVerifier::new(VERIFIER.to_owned()),
    );
    let code = authority
        .issue_oauth_code(Code {
            client_id: "client_synthetic".into(),
            redirect_uri: CALLBACK.into(),
            challenge: challenge.as_str().into(),
            resource: RESOURCE.into(),
            principal: agent.id.clone(),
            ..Code::default()
        })
        .unwrap();
    let tokens = authority
        .exchange_oauth(&exchange("authorization_code", "", &code))
        .unwrap();
    (agent, tokens)
}

fn refresh(authority: &Authority, token: &str) -> Result<Tokens, crate::Error> {
    authority.exchange_oauth(&exchange("refresh_token", token, ""))
}

fn backdate_rotation(authority: &Authority, seconds: i64) {
    authority
        .update(|d| {
            let mut rows: Vec<Refresh> = read(d, "oauth_refresh")?;
            for row in rows.iter_mut().filter(|row| row.used) {
                row.rotated_at -= seconds;
            }
            write(d, "oauth_refresh", &rows)
        })
        .unwrap();
}

fn credentials(home: &std::path::Path) -> Vec<u8> {
    std::fs::read(home.join("credentials.json")).unwrap()
}

#[test]
fn a_refresh_retry_returns_the_one_saved_successor() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (_, first) = connected(&authority);
    let next = refresh(&authority, &first.refresh_token).unwrap();
    let retried = refresh(&authority, &first.refresh_token).unwrap();
    assert_eq!(retried.refresh_token, next.refresh_token);
    assert_eq!(retried.access_token, next.access_token);
    let saved = authority.load().unwrap();
    assert_eq!(read::<Refresh>(&saved, "oauth_refresh").unwrap().len(), 2);
    assert_eq!(read::<Value>(&saved, "oauth_access").unwrap().len(), 2);
    assert!(retried.expires_in > 3500 && retried.expires_in <= 3600);
}

#[test]
fn a_replayed_token_shares_one_chain_so_theft_is_detected() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (agent, first) = connected(&authority);
    let legitimate = refresh(&authority, &first.refresh_token).unwrap();
    let stolen = refresh(&authority, &first.refresh_token).unwrap();
    refresh(&authority, &stolen.refresh_token).unwrap();
    backdate_rotation(&authority, 61);
    assert!(refresh(&authority, &legitimate.refresh_token).is_err());
    assert!(
        authority.principal(&agent.id).is_err(),
        "the other holder's use of the shared successor revokes the connector"
    );
}

#[test]
fn a_spent_token_outside_the_retry_window_still_revokes() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (agent, first) = connected(&authority);
    refresh(&authority, &first.refresh_token).unwrap();
    backdate_rotation(&authority, 61);
    assert!(refresh(&authority, &first.refresh_token).is_err());
    assert!(authority.principal(&agent.id).is_err());
}

#[test]
fn no_usable_successor_token_is_stored() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (_, first) = connected(&authority);
    let next = refresh(&authority, &first.refresh_token).unwrap();
    let stored = String::from_utf8(credentials(home.path())).unwrap();
    assert!(!stored.contains(&next.refresh_token));
    assert!(!stored.contains(&next.access_token));
}

#[test]
fn the_retry_secret_is_dropped_once_its_window_ends() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (_, first) = connected(&authority);
    refresh(&authority, &first.refresh_token).unwrap();
    let saved = authority.load().unwrap();
    let rows: Vec<Refresh> = read(&saved, "oauth_refresh").unwrap();
    assert_ne!(rows.iter().find(|row| row.used).unwrap().retry_salt, "");
    backdate_rotation(&authority, 61);
    let (_, later) = connected(&authority);
    refresh(&authority, &later.refresh_token).unwrap();
    let saved = authority.load().unwrap();
    let rows: Vec<Refresh> = read(&saved, "oauth_refresh").unwrap();
    let cutoff = jiff::Timestamp::now().as_second() - 60;
    assert!(
        rows.iter()
            .filter(|row| row.used && row.rotated_at < cutoff)
            .all(|row| row.retry_salt.is_empty())
    );
}

#[test]
fn a_token_spent_before_this_version_is_refused_without_revoking_in_the_window() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (agent, first) = connected(&authority);
    refresh(&authority, &first.refresh_token).unwrap();
    authority
        .update(|d| {
            let mut rows: Vec<Refresh> = read(d, "oauth_refresh")?;
            for row in &mut rows {
                row.retry_salt.clear();
            }
            write(d, "oauth_refresh", &rows)
        })
        .unwrap();
    assert!(refresh(&authority, &first.refresh_token).is_err());
    assert!(authority.principal(&agent.id).unwrap().active());
}

#[test]
fn registration_refuses_oversized_metadata() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let long = format!("https://connector.example/{}", "a".repeat(2000));
    let many = vec![long.clone(), long.clone(), long];
    assert!(matches!(
        authority.register_oauth(client("Large", &many)),
        Err(crate::Error::Invalid)
    ));
    assert!(
        authority
            .register_oauth(client("Claude", &[CALLBACK.to_owned()]))
            .is_ok()
    );
}

#[test]
fn anonymous_registrations_cannot_occupy_the_store() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let approved = authority
        .register_oauth(client("Approved", &[CALLBACK.to_owned()]))
        .unwrap();
    authority
        .update(|d| {
            let mut rows: Vec<Client> = read(d, "oauth_clients")?;
            for row in &mut rows {
                row.used = true;
            }
            write(d, "oauth_clients", &rows)
        })
        .unwrap();
    let mut last = String::new();
    for _ in 0..100 {
        last = authority
            .register_oauth(client("Flood", &[CALLBACK.to_owned()]))
            .unwrap()
            .client_id;
    }
    let saved = authority.load().unwrap();
    let rows: Vec<Client> = read(&saved, "oauth_clients").unwrap();
    assert!(rows.iter().filter(|row| !row.used).count() <= 64);
    assert!(rows.iter().any(|row| row.client_id == approved.client_id));
    assert!(rows.iter().any(|row| row.client_id == last));
}

#[test]
fn unused_registrations_expire_within_an_hour() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let old = authority
        .register_oauth(client("Old", &[CALLBACK.to_owned()]))
        .unwrap();
    authority
        .update(|d| {
            let mut rows: Vec<Client> = read(d, "oauth_clients")?;
            for row in &mut rows {
                row.client_id_issued_at -= 3601;
            }
            write(d, "oauth_clients", &rows)
        })
        .unwrap();
    authority
        .register_oauth(client("New", &[CALLBACK.to_owned()]))
        .unwrap();
    let saved = authority.load().unwrap();
    let rows: Vec<Client> = read(&saved, "oauth_clients").unwrap();
    assert!(!rows.iter().any(|row| row.client_id == old.client_id));
}

#[test]
fn using_an_access_token_does_not_rewrite_the_credential_document() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (_, tokens) = connected(&authority);
    let first = authority.authenticate_oauth(&tokens.access_token).unwrap();
    assert!(first.extra.contains_key("last_used"));
    let before = credentials(home.path());
    for _ in 0..3 {
        let again = authority.authenticate_oauth(&tokens.access_token).unwrap();
        assert!(again.extra.contains_key("last_used"));
    }
    assert_eq!(credentials(home.path()), before);
}

#[test]
fn using_an_agent_token_does_not_rewrite_the_credential_document() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let scopes = [Grant::parse("calendar:read").unwrap()];
    let (_, secret) = authority
        .create_oauth_agent("Static", "approve", &scopes, false)
        .unwrap();
    authority.authenticate_token(&secret).unwrap();
    let before = credentials(home.path());
    for _ in 0..3 {
        authority.authenticate_token(&secret).unwrap();
    }
    assert_eq!(credentials(home.path()), before);
}

#[test]
fn stale_last_used_is_still_refreshed() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let (agent, tokens) = connected(&authority);
    authority.authenticate_oauth(&tokens.access_token).unwrap();
    authority
        .update(|d| {
            let stored = d.principals.iter_mut().find(|p| p.id == agent.id).unwrap();
            stored
                .extra
                .insert("last_used".into(), "2020-01-01T00:00:00Z".into());
            Ok(())
        })
        .unwrap();
    authority.authenticate_oauth(&tokens.access_token).unwrap();
    let stored = authority.principal(&agent.id).unwrap();
    assert_ne!(stored.extra["last_used"], "2020-01-01T00:00:00Z");
}

#[test]
fn client_names_lose_line_breaks_and_invisible_controls() {
    let shown = |name: &str| display_name(&client(name, &[CALLBACK.to_owned()]));
    assert_eq!(
        shown("Claude\nCode 1234\r\nmessages"),
        "Claude Code 1234 messages"
    );
    assert_eq!(shown("Clau\u{202e}de\u{200b}\u{2066}"), "Claude");
    assert_eq!(shown("Cla\u{2028}ude\u{85}x\u{e0041}"), "Cla ude x");
    assert_eq!(shown("  \n\t "), "claude.ai");
    assert_eq!(shown("\u{202e}\u{200b}"), "claude.ai");
    assert_eq!(shown(&"x".repeat(500)).chars().count(), 64);
    assert_eq!(shown("Claude"), "Claude");
    assert_eq!(shown("ChatGPT Connector — été"), "ChatGPT Connector — été");
}

#[test]
fn registered_names_are_stored_normalized() {
    let home = tempfile::tempdir().unwrap();
    let authority = Authority::open(home.path()).unwrap();
    let saved = authority
        .register_oauth(client("Claude\nreal", &[CALLBACK.to_owned()]))
        .unwrap();
    assert_eq!(saved.client_name, "Claude real");
    let legacy = authority
        .update(|d| {
            let mut rows: Vec<Client> = read(d, "oauth_clients")?;
            rows[0].client_name = "Old\nname".into();
            write(d, "oauth_clients", &rows)?;
            Ok(rows[0].client_id.clone())
        })
        .unwrap();
    assert_eq!(
        authority
            .oauth_client(&legacy, CALLBACK)
            .unwrap()
            .client_name,
        "Old name"
    );
}
