#![allow(clippy::expect_used, clippy::panic, clippy::default_trait_access)]
use super::*;
use crate::approval_review::{OwnerReview, Preview, phone_decision_hash};
use crate::{GatewayStore, LocalQueryStore};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Mutex as StdMutex;

#[derive(Debug)]
struct TestIdentity(Principal);
impl Identity for TestIdentity {
    fn authenticate(&self, _: &str) -> Result<Principal, Error> {
        Err(Code::Unsupported.into())
    }
    fn current(&self, p: &Principal) -> Result<Principal, Error> {
        Ok(p.clone())
    }
    fn sign(&self, _: &Principal, _: &str, _: &str) -> Result<String, Error> {
        Err(Code::Unsupported.into())
    }
    fn approval_principal(&self, _: &str, _: &str, _: &str) -> Result<Principal, Error> {
        Ok(self.0.clone())
    }
}
#[derive(Debug)]
struct TestCodec;
impl Codec for TestCodec {
    fn json(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(bytes.to_vec())
    }
    fn command_hash(&self, c: &Command) -> Result<String, Error> {
        Ok(ab_trust::fingerprint(c.id.as_bytes()))
    }
    fn fingerprint(&self, bytes: &[u8]) -> String {
        ab_trust::fingerprint(bytes)
    }
}
#[derive(Debug, Default)]
struct Seen {
    frames: StdMutex<Vec<Value>>,
    staged: StdMutex<Vec<Vec<u8>>>,
}
#[derive(Debug, Default)]
struct TestPeer(Seen);
impl ApplePeer for TestPeer {
    fn available(&self) -> bool {
        true
    }
    fn exchange(&self, f: IpcFrame) -> crate::Task<'_, IpcFrame> {
        Box::pin(async move {
            let args: Value = if f.t == "Prepare" {
                let action: Value =
                    serde_json::from_str(f.action.as_ref().ok_or(Code::Corrupt)?.get())
                        .map_err(|_| Code::Corrupt)?;
                action["args"].clone()
            } else {
                serde_json::from_str(f.args.as_ref().ok_or(Code::Corrupt)?.get())
                    .map_err(|_| Code::Corrupt)?
            };
            if let Some(path) = args["file"].as_str()
                && let Ok(bytes) = std::fs::read(path)
            {
                self.0.staged.lock().map_err(|_| Code::Corrupt)?.push(bytes);
            }
            self.0.frames.lock().map_err(|_| Code::Corrupt)?.push(args);
            Ok(if f.t == "Prepare" {
                IpcFrame {
                    t: "Rejected".into(),
                    ..Default::default()
                }
            } else {
                IpcFrame {
                    t: "QueryResult".into(),
                    result_json: Some(
                        serde_json::value::to_raw_value(&json!({"ok":true}))
                            .map_err(|_| Code::Corrupt)?,
                    ),
                    ..Default::default()
                }
            })
        })
    }
    fn captured(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Cursor, Error> {
        Err(Code::NotFound.into())
    }
    fn capture_revision(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
}
#[derive(Debug)]
struct NativeActions;
impl ActionResolver for NativeActions {
    fn serialization_key(&self, _: &Principal, c: &Command) -> Result<String, Error> {
        Ok(c.op.clone())
    }
    fn resolve(&self, _: &Principal, c: &Command) -> Result<PreparedAction, Error> {
        Ok(PreparedAction {
            action: crate::command_contract::action(c, &Default::default())?,
            expected_state: None,
            collection: String::new(),
            serialization_key: c.op.clone(),
        })
    }
    fn evidence(&self, _: &Command, _: &str, _: &IpcFrame) -> Result<OutcomeEvidence, Error> {
        Ok(OutcomeEvidence::Standalone)
    }
}

fn principal(id: &str, kind: &str, level: &str, grants: &[&str]) -> Principal {
    Principal {
        id: id.into(),
        kind: kind.into(),
        grants: grants.iter().map(|g| (*g).to_owned()).collect(),
        level: level.into(),
        resource: String::new(),
        remote: false,
        expires_at: None,
    }
}
fn owner() -> Principal {
    principal("device:owner", "device", "", &["admin"])
}
fn agent() -> Principal {
    principal(
        "agent:synthetic",
        "agent",
        "approve",
        &["messages:write", "print:write"],
    )
}

struct Fixture {
    home: tempfile::TempDir,
    gateway: Arc<LocalGateway>,
    peer: Arc<TestPeer>,
    blobs: Arc<tokio::sync::Mutex<ab_blobs::Store>>,
    store: LocalQueryStore,
}
fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home");
    let state = GatewayStore::open(home.path()).expect("state");
    let peer = Arc::new(TestPeer::default());
    let store = LocalQueryStore::open(state.clone(), home.path(), 1 << 30).expect("blobs");
    let blobs = store.blobs.clone();
    let mut gateway = LocalGateway::new(
        Arc::new(state.clone()),
        Arc::new(TestIdentity(agent())),
        Arc::new(TestCodec),
        peer.clone(),
        Arc::new(NativeActions),
    );
    gateway.query_store = Some(Arc::new(LocalQueryStore {
        state,
        blobs: blobs.clone(),
    }));
    Fixture {
        home,
        gateway: Arc::new(gateway),
        peer,
        blobs,
        store,
    }
}
fn query(op: &str, args: &Value) -> NativeQuery {
    NativeQuery {
        id: String::new(),
        op: op.into(),
        args: serde_json::value::to_raw_value(args).expect("args"),
        deadline_ms: 5000,
    }
}
fn command(id: &str, op: &str, args: &Value) -> Command {
    serde_json::from_value(json!({"command_id": id, "op": op, "args": args})).expect("command")
}
fn state_of(raw: &RawValue) -> String {
    let value: Value = serde_json::from_str(raw.get()).expect("record");
    value["state"].as_str().unwrap_or_default().to_owned()
}
fn queries_reaching_native(f: &Fixture) -> usize {
    f.peer.0.frames.lock().expect("frames").len()
}

#[tokio::test]
async fn every_agent_capture_requires_a_fresh_owner_approval() {
    for level in ["read", "approve", "full"] {
        for (op, scope, args) in [
            ("capture.screenshot", "capture:write", json!({})),
            ("capture.camera_photo", "capture:write", json!({})),
            (
                "capture.record_audio",
                "capture:write",
                json!({"seconds": 1}),
            ),
            ("mac.screen_record", "mac:write", json!({"seconds": 1})),
        ] {
            let f = fixture();
            let p = principal("agent:capture", "agent", level, &[scope]);
            for id in ["capture_one", "capture_two"] {
                let answer = f.gateway.submit(&p, command(id, op, &args)).await;
                if level == "read" {
                    assert_eq!(
                        answer.expect_err("read cannot capture").code,
                        Code::Unauthorized
                    );
                } else {
                    assert_eq!(state_of(&answer.expect("pending")), "awaiting_approval");
                    let row = f.gateway.state.command(id).expect("row");
                    let review = f.gateway.approval_review(&row).await.expect("review");
                    f.gateway
                        .decide(
                            &owner(),
                            id,
                            false,
                            &OwnerReview {
                                binding: &review.binding,
                                accepted_unpreviewable: false,
                            },
                        )
                        .await
                        .expect("declined");
                }
                assert_eq!(queries_reaching_native(&f), 0, "{level} {op}");
            }
        }
    }
}

const DOCUMENT_QUERIES: [(&str, &str); 4] = [
    ("pdf.text", "pdf:read"),
    ("pdf.search", "pdf:read"),
    ("vision.ocr", "vision:read"),
    ("audio.transcribe", "audio:read"),
];

#[tokio::test]
async fn a_document_query_never_takes_a_caller_path() {
    let f = fixture();
    let secret = f.home.path().join("outside.pdf");
    std::fs::write(&secret, b"%PDF-synthetic").expect("write");
    let path = secret.to_str().expect("path");
    for (op, scope) in DOCUMENT_QUERIES {
        let p = principal("agent:reader", "agent", "read", &[scope]);
        for args in [
            json!({"file": path, "allow_file": true}),
            json!({"file": path}),
            json!({"FILE": path, "ALLOW_FILE": true}),
        ] {
            let error = f
                .gateway
                .query(&p, query(op, &args))
                .await
                .expect_err(&format!("{op} {args}"));
            assert_eq!(error.code, Code::Invalid, "{op} {args}");
        }
    }
    assert_eq!(queries_reaching_native(&f), 0, "a path reached the Helper");
}

#[tokio::test]
async fn a_document_query_refuses_the_retired_opt_in_flag() {
    let f = fixture();
    let p = principal("agent:reader", "agent", "read", &["pdf:read"]);
    let error = f
        .gateway
        .query(
            &p,
            query(
                "pdf.text",
                &json!({"data_base64": "JVBERg==", "allow_file": true}),
            ),
        )
        .await
        .expect_err("allow_file is not an input");
    assert_eq!(error.code, Code::Invalid);
    assert_eq!(queries_reaching_native(&f), 0);
}

#[tokio::test]
async fn inline_documents_still_reach_the_helper_and_large_ones_are_staged_privately() {
    let f = fixture();
    for (op, scope) in DOCUMENT_QUERIES {
        let p = principal("agent:reader", "agent", "read", &[scope]);
        f.gateway
            .query(&p, query(op, &json!({"data_base64": "JVBERg=="})))
            .await
            .expect(op);
    }
    let large = STANDARD.encode(vec![b'x'; 200 << 10]);
    let p = principal("agent:reader", "agent", "read", &["audio:read"]);
    f.gateway
        .query(
            &p,
            query("audio.transcribe", &json!({"data_base64": large})),
        )
        .await
        .expect("large");
    let frames = f.peer.0.frames.lock().expect("frames");
    assert!(frames[frames.len() - 1]["data_base64"].is_null());
    assert_eq!(f.peer.0.staged.lock().expect("staged")[0].len(), 200 << 10);
}

#[tokio::test]
async fn an_owned_query_blob_is_the_way_to_read_a_files_result() {
    let f = fixture();
    let bytes = b"%PDF-synthetic-files-read".to_vec();
    let (digest, _) = f.blobs.lock().await.stage(&bytes).expect("stage");
    f.store
        .state
        .with(|s| {
            s.save_query_blobs(
                "agent:reader",
                "qry_files",
                "files:read",
                &json!([{"digest": digest.as_str(), "size": bytes.len()}]),
            )
        })
        .expect("grant");
    let reference = json!({"blob": {"query_id": "qry_files", "digest": digest.as_str()}});
    let allowed = principal("agent:reader", "agent", "read", &["pdf:read", "files:read"]);
    f.gateway
        .query(&allowed, query("pdf.text", &reference))
        .await
        .expect("a real Files grant reads the blob");
    let frames = f.peer.0.frames.lock().expect("frames").clone();
    assert_eq!(frames[0]["data_base64"], STANDARD.encode(&bytes));
    let no_files = principal("agent:reader", "agent", "read", &["pdf:read"]);
    let error = f
        .gateway
        .query(&no_files, query("pdf.text", &reference))
        .await
        .expect_err("no Files grant");
    assert_eq!(error.code, Code::Forbidden);
    let stranger = principal("agent:other", "agent", "read", &["pdf:read", "files:read"]);
    let error = f
        .gateway
        .query(&stranger, query("pdf.text", &reference))
        .await
        .expect_err("another connector");
    assert_eq!(error.code, Code::Forbidden);
}

const ANNOTATION: &str =
    r#"[{"page":1,"type":"note","text":"synthetic","x":0,"y":0,"width":20,"height":20}]"#;

#[tokio::test]
async fn pdf_annotate_never_takes_a_caller_path() {
    let f = fixture();
    let annotations: Value = serde_json::from_str(ANNOTATION).expect("annotations");
    let p = principal("agent:full", "agent", "full", &["pdf:write"]);
    let error = f
        .gateway
        .submit(
            &p,
            command(
                "cmd_annotate_path",
                "pdf.annotate",
                &json!({"file": "/etc/hosts", "allow_file": true, "annotations": annotations}),
            ),
        )
        .await
        .expect_err("path");
    assert_eq!(error.code, Code::Invalid);
    assert_eq!(queries_reaching_native(&f), 0);
}

#[tokio::test]
async fn a_stored_annotate_with_a_path_is_refused_at_dispatch() {
    let f = fixture();
    let document = f
        .store
        .document_input(
            &owner(),
            &serde_json::value::to_raw_value(&json!({"file": "/etc/hosts", "allow_file": true}))
                .expect("args"),
        )
        .await;
    assert_eq!(document.expect_err("path").code, Code::Invalid);
}

#[tokio::test]
async fn print_document_never_takes_a_caller_path() {
    let f = fixture();
    let p = principal("agent:full", "agent", "full", &["print:write"]);
    let error = f
        .gateway
        .submit(
            &p,
            command(
                "cmd_print_path",
                "print.document",
                &json!({"file": "/etc/hosts", "printer": "PDF"}),
            ),
        )
        .await
        .expect_err("path");
    assert_eq!(error.code, Code::Invalid);
    assert_eq!(queries_reaching_native(&f), 0);
}

async fn wait_for_frame(f: &Fixture) -> Value {
    for _ in 0..200 {
        if let Some(frame) = f.peer.0.frames.lock().expect("frames").first().cloned() {
            return frame;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no frame reached the peer");
}

#[tokio::test]
async fn print_document_stages_inline_bytes_for_the_session() {
    let f = fixture();
    let p = principal("device:owner", "device", "", &["admin"]);
    f.gateway
        .submit(
            &p,
            command(
                "cmd_print_inline",
                "print.document",
                &json!({"data_base64": "JVBERi1zeW50aGV0aWM=", "printer": "PDF"}),
            ),
        )
        .await
        .expect("inline print");
    let args = wait_for_frame(&f).await;
    let file = args["file"].as_str().expect("staged path");
    assert!(file.contains("ab-document-"), "{file}");
    assert!(
        args["data_base64"].is_null() && args["blob"].is_null(),
        "{args}"
    );
    assert_eq!(args["printer"], "PDF");
    assert_eq!(
        f.peer.0.staged.lock().expect("staged")[0],
        b"%PDF-synthetic"
    );
}

#[tokio::test]
async fn print_document_resolves_only_an_authorized_blob() {
    let f = fixture();
    let bytes = b"%PDF-synthetic-print".to_vec();
    let (digest, _) = f.blobs.lock().await.stage(&bytes).expect("stage");
    f.store
        .state
        .with(|s| {
            s.save_query_blobs(
                "agent:full",
                "qry_print",
                "files:read",
                &json!([{"digest": digest.as_str(), "size": bytes.len()}]),
            )
        })
        .expect("grant");
    let args =
        json!({"blob": {"query_id": "qry_print", "digest": digest.as_str()}, "printer": "PDF"});
    let denied = principal("agent:full", "agent", "full", &["print:write"]);
    let error = f
        .gateway
        .submit(
            &denied,
            command("cmd_print_denied", "print.document", &args),
        )
        .await
        .expect_err("no Files grant");
    assert_eq!(error.code, Code::Forbidden);
    assert_eq!(queries_reaching_native(&f), 0);

    let allowed = principal(
        "agent:full",
        "agent",
        "full",
        &["print:write", "files:read"],
    );
    f.gateway
        .submit(
            &allowed,
            command("cmd_print_allowed", "print.document", &args),
        )
        .await
        .expect("authorized print");
    let sent = wait_for_frame(&f).await;
    assert!(
        sent["file"]
            .as_str()
            .expect("file")
            .contains("ab-document-")
    );
    assert_eq!(f.peer.0.staged.lock().expect("staged")[0], bytes);
}

fn send_args(files: &Value) -> Value {
    json!({"chat_id": "chat_synthetic", "text": "see attached", "files": files})
}

async fn pending_with_upload(f: &Fixture, name: &str, bytes: &[u8]) -> (String, CommandRecord) {
    let status = f
        .blobs
        .lock()
        .await
        .stage_message("agent:synthetic", bytes)
        .expect("upload");
    let id = format!("cmd_{}", ulid::Ulid::new());
    let raw = f
        .gateway
        .submit(
            &agent(),
            command(
                &id,
                "messages.send",
                &send_args(&json!([{"name": name, "upload_id": status.upload_id}])),
            ),
        )
        .await
        .expect("submit");
    assert_eq!(state_of(&raw), "awaiting_approval");
    let record = f.gateway.state.command(&id).expect("row");
    (id, record)
}

#[tokio::test]
async fn the_owner_sees_what_an_attachment_really_is() {
    let f = fixture();
    let marker = b"synthetic marker text, not a pdf";
    let (_, record) = pending_with_upload(&f, "harmless-report.pdf", marker).await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert_eq!(review.attachments.len(), 1);
    let attachment = &review.attachments[0];
    assert_eq!(attachment.name, "harmless-report.pdf");
    assert_eq!(attachment.media_type, "text/plain");
    assert_eq!(attachment.size, marker.len() as u64);
    assert_eq!(attachment.digest, ab_trust::fingerprint(marker));
    assert_eq!(attachment.preview, Preview::Text);
    assert!(!review.requires_consent);
    assert_eq!(review.binding.len(), 64);
    let (shown, bytes) = f
        .gateway
        .approval_attachment(&record.command.id, 0)
        .await
        .expect("preview");
    assert_eq!(bytes, marker);
    assert_eq!(shown.digest, attachment.digest);
}

#[tokio::test]
async fn an_inline_attachment_is_reviewed_from_its_decoded_bytes() {
    let f = fixture();
    let id = "cmd_inline_review";
    f.gateway
        .submit(
            &agent(),
            command(
                id,
                "messages.send",
                &send_args(
                    &json!([{"name": "photo.png", "data": STANDARD.encode(b"inline text")}]),
                ),
            ),
        )
        .await
        .expect("submit");
    let record = f.gateway.state.command(id).expect("row");
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert_eq!(review.attachments[0].media_type, "text/plain");
    assert_eq!(
        review.attachments[0].digest,
        ab_trust::fingerprint(b"inline text")
    );
    let shown = crate::approval_review::display_arguments(&record).expect("arguments");
    assert!(!shown.contains(&STANDARD.encode(b"inline text")), "{shown}");
}

#[tokio::test]
async fn an_image_previews_but_a_scriptable_or_binary_file_does_not() {
    let f = fixture();
    let png = b"\x89PNG\r\n\x1a\nsynthetic";
    let (_, record) = pending_with_upload(&f, "a.png", png).await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert_eq!(review.attachments[0].media_type, "image/png");
    assert_eq!(review.attachments[0].preview, Preview::Image);
    assert!(
        review.requires_consent,
        "an image signature says nothing about what the owner will see"
    );
    for (name, bytes, media) in [
        ("a.pdf", &b"%PDF-1.7 synthetic"[..], "application/pdf"),
        (
            "a.exe",
            &b"MZ\0\0\xff\xfe synthetic"[..],
            "application/octet-stream",
        ),
    ] {
        let (_, record) = pending_with_upload(&f, name, bytes).await;
        let review = f.gateway.approval_review(&record).await.expect("review");
        assert_eq!(review.attachments[0].media_type, media, "{name}");
        assert_eq!(
            review.attachments[0].preview,
            Preview::DownloadOnly,
            "{name}"
        );
        assert!(review.requires_consent, "{name}");
    }
    let (_, record) = pending_with_upload(&f, "a.html", b"<script>alert(1)</script>").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert_eq!(review.attachments[0].media_type, "text/plain");
}

#[tokio::test]
async fn the_phone_warns_unless_it_shows_the_whole_attachment() {
    let f = fixture();
    let warning = "You haven't seen all of this file here";
    let summary = |review: &crate::approval_review::ApprovalReview| {
        crate::approval_review::phone_summary(review).join("\n")
    };
    let (_, record) = pending_with_upload(&f, "short.txt", b"all of it").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    let shown = summary(&review);
    assert!(shown.contains("Starts with: all of it"), "{shown}");
    assert!(!shown.contains(warning), "{shown}");
    assert!(!review.requires_consent);

    for (name, altered) in [
        ("tabs.txt", &b"payee\tamount\nAlice\t100"[..]),
        ("blank.txt", &b" \n\t "[..]),
    ] {
        let (_, record) = pending_with_upload(&f, name, altered).await;
        let review = f.gateway.approval_review(&record).await.expect("review");
        assert!(summary(&review).contains(warning), "{name}");
    }
    let (_, record) = pending_with_upload(&f, "line.txt", b"one line\n").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert!(
        !summary(&review).contains(warning),
        "a final newline hides nothing"
    );

    let long = "synthetic ".repeat(100);
    let (_, record) = pending_with_upload(&f, "long.txt", long.as_bytes()).await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert!(summary(&review).contains(warning), "truncated text");

    let (_, record) = pending_with_upload(&f, "a.png", b"\x89PNG\r\n\x1a\nsynthetic").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert!(summary(&review).contains(warning), "image");

    let (_, record) = pending_with_upload(&f, "a.bin", b"\0\x01synthetic").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert!(summary(&review).contains(warning), "binary");
}

#[tokio::test]
async fn an_approval_must_name_the_attachments_it_reviewed() {
    let f = fixture();
    let (id, record) = pending_with_upload(&f, "note.txt", b"synthetic note").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    for binding in ["", "stale"] {
        let error = f
            .gateway
            .decide(
                &owner(),
                &id,
                true,
                &OwnerReview {
                    binding,
                    accepted_unpreviewable: false,
                },
            )
            .await
            .expect_err("unreviewed");
        assert_eq!(error.code, Code::Conflict, "{binding:?}");
        assert_eq!(
            f.gateway.state.command(&id).expect("row").state,
            State::AwaitingApproval
        );
    }
    let decided = f
        .gateway
        .decide(
            &owner(),
            &id,
            true,
            &OwnerReview {
                binding: &review.binding,
                accepted_unpreviewable: false,
            },
        )
        .await
        .expect("approved");
    assert_ne!(state_of(&decided), "awaiting_approval");
}

#[tokio::test]
async fn an_unpreviewable_attachment_needs_explicit_consent() {
    let f = fixture();
    let (id, record) = pending_with_upload(&f, "a.bin", b"\0\x01\x02synthetic").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    let error = f
        .gateway
        .decide(
            &owner(),
            &id,
            true,
            &OwnerReview {
                binding: &review.binding,
                accepted_unpreviewable: false,
            },
        )
        .await
        .expect_err("no consent");
    assert_eq!(error.code, Code::Conflict);
    let decided = f
        .gateway
        .decide(
            &owner(),
            &id,
            true,
            &OwnerReview {
                binding: &review.binding,
                accepted_unpreviewable: true,
            },
        )
        .await
        .expect("consented");
    assert_ne!(state_of(&decided), "awaiting_approval");
}

#[tokio::test]
async fn the_phone_decision_is_bound_to_the_attachments() {
    let f = fixture();
    let (id, record) = pending_with_upload(&f, "note.txt", b"synthetic note").await;
    let review = f.gateway.approval_review(&record).await.expect("review");
    let error = f
        .gateway
        .decide_phone(&id, &record.request_hash, true)
        .await
        .expect_err("hash without the attachments");
    assert_eq!(error.code, Code::Unauthorized);
    let bound = phone_decision_hash(&record.request_hash, &review.binding);
    let decided = f
        .gateway
        .decide_phone(&id, &bound, true)
        .await
        .expect("bound");
    assert_ne!(state_of(&decided), "awaiting_approval");
}

#[tokio::test]
async fn a_rejection_never_needs_a_review() {
    let f = fixture();
    let (id, record) = pending_with_upload(&f, "note.txt", b"synthetic note").await;
    let decided = f
        .gateway
        .decide(
            &owner(),
            &id,
            false,
            &OwnerReview {
                binding: "",
                accepted_unpreviewable: false,
            },
        )
        .await
        .expect("rejected");
    assert_eq!(state_of(&decided), "rejected_by_owner");
    let (id, record2) = pending_with_upload(&f, "other.txt", b"other synthetic note").await;
    let decided = f
        .gateway
        .decide_phone(&id, &record2.request_hash, false)
        .await
        .expect("phone rejection");
    assert_eq!(state_of(&decided), "rejected_by_owner");
    assert_eq!(record.state, State::AwaitingApproval);
}

#[tokio::test]
async fn a_command_without_attachments_decides_as_before() {
    let f = fixture();
    let id = "cmd_plain_decision";
    f.gateway
        .submit(
            &agent(),
            command(
                id,
                "messages.send",
                &json!({"chat_id": "chat_synthetic", "text": "hello"}),
            ),
        )
        .await
        .expect("submit");
    let record = f.gateway.state.command(id).expect("row");
    let review = f.gateway.approval_review(&record).await.expect("review");
    assert!(review.attachments.is_empty() && review.binding.is_empty());
    let decided = f
        .gateway
        .decide_phone(id, &record.request_hash, true)
        .await
        .expect("plain");
    assert_ne!(state_of(&decided), "awaiting_approval");
}

#[tokio::test]
async fn a_preview_is_only_for_a_pending_approval() {
    let f = fixture();
    let (id, _) = pending_with_upload(&f, "note.txt", b"synthetic note").await;
    assert_eq!(
        f.gateway
            .approval_attachment(&id, 5)
            .await
            .expect_err("no such index")
            .code,
        Code::NotFound
    );
    f.gateway
        .decide(
            &owner(),
            &id,
            false,
            &OwnerReview {
                binding: "",
                accepted_unpreviewable: false,
            },
        )
        .await
        .expect("rejected");
    assert_eq!(
        f.gateway
            .approval_attachment(&id, 0)
            .await
            .expect_err("decided")
            .code,
        Code::NotFound
    );
}

#[test]
fn authorization_rejects_read_agents_even_with_write_grants() {
    let fixture = fixture();
    for kind in ["agent", "delegated"] {
        let reader = principal(
            "replica:peer/agent:reader",
            kind,
            "read",
            &["messages:write"],
        );
        assert_eq!(
            fixture
                .gateway
                .authorize(&reader, "messages.send")
                .expect_err("read agent")
                .code,
            Code::Unauthorized
        );
    }
    let writer = principal("token:writer", "token", "", &["messages:write"]);
    assert!(fixture.gateway.authorize(&writer, "messages.send").is_ok());
    let ungranted = principal("token:reader", "token", "", &["messages:read"]);
    assert_eq!(
        fixture
            .gateway
            .authorize(&ungranted, "messages.send")
            .expect_err("missing grant")
            .code,
        Code::Unauthorized
    );
}

#[tokio::test]
async fn approval_entry_points_reject_invalid_deciders_before_lookup() {
    let fixture = fixture();
    let review = OwnerReview {
        binding: "",
        accepted_unpreviewable: false,
    };
    assert_eq!(
        fixture
            .gateway
            .decide(&agent(), "missing", true, &review)
            .await
            .expect_err("owner required")
            .code,
        Code::Unauthorized
    );
    let device = principal("device:reader", "device", "", &["messages:read"]);
    assert_eq!(
        fixture
            .gateway
            .decide(&device, "missing", true, &review)
            .await
            .expect_err("admin required")
            .code,
        Code::Unauthorized
    );
    assert_eq!(
        fixture
            .gateway
            .decide_phone("missing", "", true)
            .await
            .expect_err("hash required")
            .code,
        Code::Invalid
    );
}
