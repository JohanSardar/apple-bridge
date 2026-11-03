#![cfg(feature = "isolated-control")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use ab_relay_control::{Control, Error, IdentityProvider, ProviderFuture, device_message, router};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ciborium::Value as C;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use p256::ecdsa::signature::Signer as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

static SOURCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
struct Fixture;
impl IdentityProvider for Fixture {
    fn authorize(&self, state: &str, challenge: &str) -> ab_relay_control::Result<String> {
        Ok(format!(
            "https://fixture.invalid?state={state}&challenge={challenge}"
        ))
    }
    fn subject<'a>(&'a self, code: &'a str, _verifier: &'a str) -> ProviderFuture<'a> {
        Box::pin(async move { Ok(format!("github:{code}")) })
    }
}
struct World {
    dir: tempfile::TempDir,
    c: Arc<Control>,
    now: i64,
}
impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let c = Arc::new(
            Control::open(
                &dir.path().join("control.sqlite"),
                &dir.path().join("external-epoch"),
            )
            .unwrap(),
        );
        Self {
            dir,
            c,
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .try_into()
                .unwrap(),
        }
    }
    async fn login(&self, id: &str, invite: &str) -> Value {
        let start = self
            .c
            .oauth_start(&Fixture, invite, "browser", self.now)
            .unwrap();
        self.c
            .oauth_finish(&Fixture, s(&start, "state"), id, "browser", self.now)
            .await
            .unwrap()
    }
    async fn request(
        &self,
        action: &str,
        input: Value,
        session: Option<&Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .uri(format!("/v1/{action}"))
            .method("POST")
            .header("host", "app.lexaai.tech")
            .header("origin", "https://app.lexaai.tech")
            .header("content-type", "application/json");
        if let Some(session) = session {
            request = request
                .header(
                    "cookie",
                    format!("__Host-ab-control={}", s(session, "session")),
                )
                .header("x-csrf-token", s(session, "csrf"));
        }
        let mut request = request.body(Body::from(input.to_string())).unwrap();
        let source = SOURCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((
                [
                    127,
                    0,
                    u8::try_from(source / 250).unwrap(),
                    u8::try_from(source % 250).unwrap(),
                ],
                3456,
            ))));
        let response = router(self.c.clone(), Arc::new(Fixture))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    async fn grant(&self, session: &Value, seed: u8, slot: u8) -> (Value, Value, SigningKey) {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let public = B64.encode(key.verifying_key().as_bytes());
        let start = self.c.start_grant(&public, slot, self.now).unwrap();
        let (_, view) = self
            .request(
                "confirm-view",
                json!({"code":start["user_code"]}),
                Some(session),
            )
            .await;
        assert_eq!(view["fingerprint"], start["fingerprint"]);
        assert_eq!(view["code"], start["user_code"]);
        assert_eq!(
            self.request(
                "confirm",
                json!({"code":start["user_code"],"fingerprint":start["fingerprint"]}),
                Some(session)
            )
            .await
            .0,
            StatusCode::OK
        );
        let poll = self
            .c
            .poll(s(&start, "polling_secret"), self.now + 5)
            .unwrap();
        (start, poll, key)
    }
    async fn device(&self, session: &Value, seed: u8, slot: u8) -> Value {
        let (start, poll, key) = self.grant(session, seed, slot).await;
        self.c
            .complete(
                s(&start, "polling_secret"),
                &sign(&key, &poll),
                self.now + 6,
            )
            .unwrap()
    }
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap()
}
fn sign(key: &SigningKey, poll: &Value) -> String {
    B64.encode(
        key.sign(&device_message(
            s(poll, "transaction"),
            s(poll, "nonce"),
            s(poll, "public_key"),
            poll["slot"].as_u64().unwrap().try_into().unwrap(),
        ))
        .to_bytes(),
    )
}

#[tokio::test]
async fn relay_account_enroll() {
    let w = World::new();
    let invite = w.c.invite(w.now + 60, 1).unwrap();
    let account = w.login("123", &invite).await;
    assert_eq!(w.login("123", "").await["account"], account["account"]);
    let start = w.c.oauth_start(&Fixture, "", "right", w.now).unwrap();
    assert!(matches!(
        w.c.oauth_finish(&Fixture, s(&start, "state"), "999", "wrong", w.now)
            .await,
        Err(Error::Forbidden)
    ));
    let key = p256::ecdsa::SigningKey::from_slice(&[7; 32]).unwrap();
    let (status, start) = w
        .request("passkey-register-start", json!({}), Some(&account))
        .await;
    assert_eq!(status, StatusCode::OK);
    let challenge = s(&start["publicKey"], "challenge");
    let client=json!({"type":"webauthn.create","challenge":challenge,"origin":"https://app.lexaai.tech","crossOrigin":false}).to_string();
    let point = key.verifying_key().to_sec1_point(false);
    let id = B64.encode(b"fixture-passkey");
    let cose = C::Map(vec![
        (C::Integer(1.into()), C::Integer(2.into())),
        (C::Integer(3.into()), C::Integer((-7).into())),
        (C::Integer((-1).into()), C::Integer(1.into())),
        (
            C::Integer((-2).into()),
            C::Bytes(point.x().unwrap().to_vec()),
        ),
        (
            C::Integer((-3).into()),
            C::Bytes(point.y().unwrap().to_vec()),
        ),
    ]);
    let mut auth = Sha256::digest(b"app.lexaai.tech").to_vec();
    auth.push(69);
    auth.extend(0u32.to_be_bytes());
    auth.extend([0u8; 16]);
    auth.extend((15u16).to_be_bytes());
    auth.extend(b"fixture-passkey");
    let mut cose_bytes = Vec::new();
    ciborium::into_writer(&cose, &mut cose_bytes).unwrap();
    auth.extend(cose_bytes);
    let att = C::Map(vec![
        (C::Text("fmt".into()), C::Text("none".into())),
        (C::Text("authData".into()), C::Bytes(auth)),
        (C::Text("attStmt".into()), C::Map(vec![])),
    ]);
    let mut object = Vec::new();
    ciborium::into_writer(&att, &mut object).unwrap();
    let (status,registered)=w.request("passkey-register-finish",json!({"transaction":start["transaction"],"id":id,"response":{"clientDataJSON":B64.encode(client),"attestationObject":B64.encode(object)}}),Some(&account)).await;
    assert_eq!(status, StatusCode::OK, "{registered}");
    assert_eq!(registered["recovery_codes"].as_array().unwrap().len(), 8);
    let (_, start) = w
        .request(
            "passkey-login-start",
            json!({"account":account["account"]}),
            None,
        )
        .await;
    let client=json!({"type":"webauthn.get","challenge":start["publicKey"]["challenge"],"origin":"https://app.lexaai.tech"}).to_string();
    let mut auth = Sha256::digest(b"app.lexaai.tech").to_vec();
    auth.push(5);
    auth.extend(1u32.to_be_bytes());
    let mut message = auth.clone();
    message.extend(Sha256::digest(client.as_bytes()));
    let signature: p256::ecdsa::Signature = key.sign(&message);
    let input = json!({"transaction":start["transaction"],"id":id,"response":{"clientDataJSON":B64.encode(client),"authenticatorData":B64.encode(auth),"signature":B64.encode(signature.to_der())}});
    assert_eq!(
        w.request("passkey-login-finish", input.clone(), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        w.request("passkey-login-finish", input, None).await.0,
        StatusCode::GONE
    );
    let recovery = registered["recovery_codes"][0].clone();
    assert_eq!(
        w.request(
            "recover",
            json!({"account":account["account"],"code":recovery}),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        w.request(
            "recover",
            json!({"account":account["account"],"code":recovery}),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn relay_device_grant() {
    let w = World::new();
    let a = w.login("1", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let (start, poll, key) = w.grant(&a, 1, 1).await;
    assert!(matches!(
        w.c.poll(s(&start, "polling_secret"), w.now + 6),
        Err(Error::SlowDown)
    ));
    assert!(
        w.c.complete(
            s(&start, "polling_secret"),
            &sign(&SigningKey::from_bytes(&[2; 32]), &poll),
            w.now + 6
        )
        .is_err()
    );
    let sig = sign(&key, &poll);
    assert!(
        w.c.complete(s(&start, "polling_secret"), &sig, w.now + 6)
            .is_ok()
    );
    assert!(matches!(
        w.c.complete(s(&start, "polling_secret"), &sig, w.now + 7),
        Err(Error::Expired)
    ));
    let (start, poll, key) = w.grant(&a, 3, 2).await;
    assert!(matches!(
        w.c.complete(s(&start, "polling_secret"), &sign(&key, &poll), w.now + 36),
        Err(Error::Expired)
    ));
    assert!(w.c.poll(s(&start, "polling_secret"), w.now + 600).is_err());
    for _ in 0..12 {
        w.c.throttle("abuser", w.now).unwrap();
    }
    assert!(matches!(
        w.c.throttle("abuser", w.now),
        Err(Error::SlowDown)
    ));
}
#[tokio::test]
async fn relay_invite_only() {
    let w = World::new();
    let start =
        w.c.oauth_start(&Fixture, "missing", "browser", w.now)
            .unwrap();
    assert!(
        w.c.oauth_finish(&Fixture, s(&start, "state"), "1", "browser", w.now)
            .await
            .is_err()
    );
    let invite = w.c.invite(w.now + 60, 1).unwrap();
    let a = w.login("1", &invite).await;
    let start =
        w.c.oauth_start(&Fixture, &invite, "browser", w.now)
            .unwrap();
    assert!(
        w.c.oauth_finish(&Fixture, s(&start, "state"), "2", "browser", w.now)
            .await
            .is_err()
    );
    w.device(&a, 1, 1).await;
    w.device(&a, 2, 2).await;
    let (start, poll, key) = w.grant(&a, 3, 1).await;
    assert!(matches!(
        w.c.complete(s(&start, "polling_secret"), &sign(&key, &poll), w.now + 6),
        Err(Error::Capacity)
    ));
    for n in 2..=25 {
        w.login(&n.to_string(), &w.c.invite(w.now + 60, 1).unwrap())
            .await;
    }
    let start =
        w.c.oauth_start(
            &Fixture,
            &w.c.invite(w.now + 60, 1).unwrap(),
            "browser",
            w.now,
        )
        .unwrap();
    assert!(matches!(
        w.c.oauth_finish(&Fixture, s(&start, "state"), "26", "browser", w.now)
            .await,
        Err(Error::Capacity)
    ));
    assert!(
        Control::open(
            &w.dir.path().join("control.sqlite"),
            &w.dir.path().join("external-epoch")
        )
        .is_err()
    );
}
#[tokio::test]
async fn relay_account_revoke() {
    let w = World::new();
    let a = w.login("1", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let b = w.login("2", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let d = w.device(&a, 1, 1).await;
    let lease =
        w.c.admit(s(&d, "device"), s(&d, "credential"), 1, w.now)
            .unwrap();
    assert_eq!(
        w.request("revoke", json!({"device":d["device"]}), Some(&b))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        w.request("suspend", json!({"device":d["device"]}), Some(&a))
            .await
            .0,
        StatusCode::OK
    );
    assert!(w.c.charge(s(&lease, "lease"), 1, 1, w.now).is_err());
    assert_eq!(
        w.request("resume", json!({"device":d["device"]}), Some(&a))
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        w.c.admit(s(&d, "device"), s(&d, "credential"), 1, w.now)
            .is_err()
    );
    assert!(
        w.c.admit(s(&d, "device"), s(&d, "credential"), 3, w.now)
            .is_ok()
    );
    let (_, rotation) = w
        .request("rotate-start", json!({"device":d["device"]}), Some(&a))
        .await;
    let key = SigningKey::from_bytes(&[8; 32]);
    let public = B64.encode(key.verifying_key().as_bytes());
    let signature = B64.encode(
        key.sign(&ab_relay_control::rotation_message(
            s(&d, "device"),
            s(&rotation, "nonce"),
            &public,
        ))
        .to_bytes(),
    );
    let (status,rotated)=w.request("rotate",json!({"device":d["device"],"nonce":rotation["nonce"],"public_key":public,"signature":signature}),Some(&a)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        w.c.admit(s(&d, "device"), s(&d, "credential"), 3, w.now)
            .is_err()
    );
    assert!(
        w.c.admit(s(&d, "device"), s(&rotated, "credential"), 4, w.now)
            .is_ok()
    );
    let began = std::time::Instant::now();
    let (status, result) = w
        .request("revoke", json!({"device":d["device"]}), Some(&a))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["committed"], true);
    assert!(
        w.c.admit(s(&d, "device"), s(&d, "credential"), 1, w.now)
            .is_err()
    );
    assert!(w.c.charge(s(&lease, "lease"), 1, 1, w.now).is_err());
    let pending = w.c.pending_disconnects().unwrap();
    assert_eq!(pending.as_array().unwrap().len(), 4);
    let read = rusqlite::Connection::open(w.dir.path().join("control.sqlite")).unwrap();
    let state: String = read
        .query_row(
            "SELECT state FROM devices WHERE id=?1",
            [s(&d, "device")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "revoked");
    for event in pending.as_array().unwrap() {
        w.c.ack_disconnect(event["id"].as_i64().unwrap()).unwrap();
    }
    assert_eq!(w.c.pending_disconnects().unwrap(), json!([]));
    assert!(began.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(
        w.request("resume", json!({"device":d["device"]}), Some(&a))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn relay_quota_account() {
    let w = World::new();
    let a = w.login("1", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let d1 = w.device(&a, 1, 1).await;
    let d2 = w.device(&a, 2, 2).await;
    let mut leases = Vec::new();
    for d in [&d1, &d2] {
        for _ in 0..32 {
            leases.push(
                w.c.admit(s(d, "device"), s(d, "credential"), 1, w.now)
                    .unwrap(),
            );
        }
        assert!(matches!(
            w.c.admit(s(d, "device"), s(d, "credential"), 1, w.now),
            Err(Error::Capacity)
        ));
    }
    w.c.charge(s(&leases[0], "lease"), 524_288, 0, w.now)
        .unwrap();
    w.c.charge(s(&leases[32], "lease"), 0, 524_288, w.now)
        .unwrap();
    assert!(matches!(
        w.c.charge(s(&leases[32], "lease"), 1, 0, w.now),
        Err(Error::Capacity)
    ));
    for lease in &leases {
        w.c.release(s(lease, "lease")).unwrap();
    }
    let lease =
        w.c.admit(s(&d1, "device"), s(&d1, "credential"), 1, w.now)
            .unwrap();
    for second in 1..10240 {
        w.c.charge(s(&lease, "lease"), 524_288, 524_288, w.now + second)
            .unwrap();
    }
    assert!(matches!(
        w.c.charge(s(&lease, "lease"), 1, 0, w.now + 10241),
        Err(Error::Capacity)
    ));
    let next_month = w.now + 40 * 86400;
    let lease =
        w.c.admit(s(&d2, "device"), s(&d2, "credential"), 1, next_month)
            .unwrap();
    w.c.charge(s(&lease, "lease"), 1, 1, next_month).unwrap();
}
#[tokio::test]
async fn relay_log_redaction() {
    let w = World::new();
    let seed = "SEEDED-SECRET-never-log";
    let (status, body) = w
        .request(
            "device-complete",
            json!({"polling_secret":seed,"signature":seed}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::GONE);
    assert!(!body.to_string().contains(seed));
    assert!(!format!("{:?}", w.c).contains(seed));
    let github = ab_relay_control::Github::new(seed.into(), seed.into()).unwrap();
    assert!(!format!("{github:?}").contains(seed));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ab-relay-control"))
        .env_remove("AB_RELAY_CONTROL_ISOLATED")
        .env("AB_RELAY_GITHUB_CLIENT_SECRET", seed)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains(seed));
    let request = Request::builder()
        .uri("/")
        .header("host", "ab.lexaai.tech")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(w.c.clone(), Arc::new(Fixture))
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let request = Request::builder()
        .uri("/")
        .header("host", "app.lexaai.tech")
        .header("origin", "https://other.ab.lexaai.tech")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(w.c.clone(), Arc::new(Fixture))
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn relay_restore_fence() {
    let w = World::new();
    let a = w.login("1", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let d = w.device(&a, 1, 1).await;
    let backup = w.dir.path().join("backup.sqlite");
    let db = rusqlite::Connection::open(w.dir.path().join("control.sqlite")).unwrap();
    db.execute("VACUUM INTO ?1", [backup.to_str().unwrap()])
        .unwrap();
    drop(db);
    assert_eq!(
        w.request("revoke", json!({"device":d["device"]}), Some(&a))
            .await
            .0,
        StatusCode::OK
    );
    let World { dir, c, .. } = w;
    drop(c);
    assert!(
        Control::open(
            &dir.path().join("control.sqlite"),
            &dir.path().join("external-epoch")
        )
        .is_ok()
    );
    std::fs::copy(backup, dir.path().join("control.sqlite")).unwrap();
    assert!(matches!(
        Control::open(
            &dir.path().join("control.sqlite"),
            &dir.path().join("external-epoch")
        ),
        Err(Error::Fenced)
    ));
}

#[tokio::test]
async fn grant_slot_race() {
    let w = World::new();
    let account = w.login("1", &w.c.invite(w.now + 60, 1).unwrap()).await;
    let mut jobs = Vec::new();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    for seed in [1, 2] {
        let (start, poll, key) = w.grant(&account, seed, 1).await;
        let control = w.c.clone();
        let time = w.now + 6;
        let barrier = barrier.clone();
        jobs.push(std::thread::spawn(move || {
            barrier.wait();
            control
                .complete(s(&start, "polling_secret"), &sign(&key, &poll), time)
                .is_ok()
        }));
    }
    assert_eq!(
        jobs.into_iter()
            .filter_map(|job| job.join().ok())
            .filter(|passed| *passed)
            .count(),
        1
    );
}
#[tokio::test]
async fn policy_transport_closed_by_default() {
    let w = World::new();
    let token = B64.encode([9; 32]);
    let policy = ab_relay_control::policy_router(w.c.clone(), &token).unwrap();
    let request = Request::builder()
        .uri("/v1/policy/disconnects")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        policy.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let request = Request::builder()
        .uri("/v1/policy/disconnects")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        policy.oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
}
