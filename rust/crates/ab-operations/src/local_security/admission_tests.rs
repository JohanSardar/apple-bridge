use super::*;
use crate::{CommandTarget, ForwardReply, GatewayRoute, GatewayTransport, Service, StateReads};

fn service(f: &Fixture) -> Service {
    Service {
        store: Arc::new(StateReads(f.store.state.0.clone(), f.blobs.clone())),
        identity: f.gateway.identity.clone(),
        codec: f.gateway.codec.clone(),
        target: CommandTarget::Local(f.gateway.clone()),
    }
}

#[derive(Debug)]
struct LostReply(Arc<LocalGateway>, Principal);

impl GatewayTransport for LostReply {
    fn connected(&self) -> bool {
        true
    }

    fn exchange<'a>(
        &'a self,
        _: &'a GatewayRoute,
        _: String,
        body: Vec<u8>,
    ) -> crate::Task<'a, ForwardReply> {
        Box::pin(async move {
            let envelope: Value = serde_json::from_slice(&body).expect("forwarded request");
            if envelope["method"] == "submit" {
                let request =
                    serde_json::from_value(envelope["command"].clone()).expect("forwarded command");
                match self.0.submit(&self.1, request).await {
                    Ok(_) => {}
                    Err(error) if error.code == Code::IdReused => {}
                    Err(error) => return Err(error),
                }
                return Err(Code::Unavailable.into());
            }
            assert_eq!(envelope["method"], "status");
            let query = serde_json::from_value(envelope["query"].clone()).expect("status query");
            Ok(ForwardReply {
                result: self.0.status(&self.1, &query).await?,
                lookup_only: false,
            })
        })
    }
}

#[tokio::test]
async fn status_recovers_a_lost_submit_reply_only_for_the_same_request_hash() {
    let f = fixture();
    let p = agent();
    let mut service = service(&f);
    service.target = CommandTarget::Remote(Arc::new(crate::RemoteGateway {
        route: GatewayRoute::Feed,
        identity: service.identity.clone(),
        store: service.store.clone(),
        codec: service.codec.clone(),
        transport: Arc::new(LostReply(f.gateway.clone(), p.clone())),
    }));
    let request = "lost-reply";
    let id = crate::scoped_command_id(&p.id, request);
    let original = command(
        &id,
        "messages.send",
        &json!({"chat_id":"chat_synthetic","text":"first"}),
    );
    let reply = match &service.target {
        CommandTarget::Remote(remote) => remote
            .submit(&p, original.clone())
            .await
            .expect("recover response"),
        CommandTarget::Local(_) => panic!("expected remote"),
    };
    assert_eq!(state_of(&reply), "awaiting_approval");
    assert_eq!(
        service
            .invoke(&p, "commands.status", br#"{"request_id":"lost-reply"}"#)
            .await
            .expect("scoped status")
            .get(),
        reply.get(),
    );
    let legacy = command(
        "legacy-reply",
        "messages.send",
        &json!({"chat_id":"chat_synthetic","text":"first"}),
    );
    let mut legacy_record: CommandRecord = serde_json::from_str(reply.get()).expect("record");
    legacy_record.request_hash = f.gateway.codec.command_hash(&legacy).expect("hash");
    legacy_record.command = legacy.clone();
    let legacy_receipt = raw(&f
        .gateway
        .state
        .transition(None, legacy_record)
        .expect("legacy row"))
    .expect("legacy receipt");
    assert_eq!(
        f.gateway
            .submit(&p, legacy)
            .await
            .expect("legacy replay")
            .get(),
        legacy_receipt.get()
    );
    assert_eq!(
        service
            .request_status(&p, "legacy-reply", false)
            .await
            .expect("legacy status")
            .get(),
        legacy_receipt.get(),
    );
    let mut changed = original;
    changed.args = raw(&json!({"chat_id":"chat_synthetic","text":"different"})).expect("args");
    let CommandTarget::Remote(remote) = &service.target else {
        panic!("expected remote");
    };
    assert_eq!(
        remote
            .submit(&p, changed)
            .await
            .expect_err("reject changed payload")
            .code,
        Code::IdReused
    );
    assert_eq!(queries_reaching_native(&f), 0);
}

#[tokio::test]
async fn photo_ids_bind_the_original_recipient_before_resolution_and_staging() {
    let f = fixture();
    let p = principal(
        "agent:photos",
        "agent",
        "approve",
        &["photos:read", "messages:write"],
    );
    let service = service(&f);
    let send = |asset| {
        command(
            "",
            "messages.send",
            &json!({"to":"+12025550100","asset_ids":[asset]}),
        )
    };
    let receipt = service
        .submit_photo_message(&p, "photo-request", send("asset-a"), |ids| {
            let blobs = &f.blobs;
            let principal = &p.id;
            Box::pin(async move {
                assert_eq!(ids, ["asset-a"]);
                let staged = blobs
                    .lock()
                    .await
                    .stage_message(principal, b"synthetic photo upload")
                    .expect("upload");
                Ok(crate::local::StagedPhotoMessage {
                    recipient: serde_json::Map::from_iter([
                        ("handles".into(), json!(["+12025550100"])),
                        ("service".into(), json!("iMessage")),
                    ]),
                    files: vec![StagedPhoto {
                        name: "synthetic.jpg".into(),
                        upload_id: staged.upload_id,
                    }],
                })
            })
        })
        .await
        .expect("admitted");
    let persisted: Value = serde_json::from_str(receipt.get()).expect("receipt");
    assert!(persisted["command"]["args"].get("asset_ids").is_none());
    assert!(persisted["command"]["args"].get("to").is_none());
    assert_eq!(
        persisted["command"]["args"]["handles"],
        json!(["+12025550100"])
    );
    let fail_if_staged = |_| panic!("a request with this ID is already bound");
    let replay = service
        .submit_photo_message(&p, "photo-request", send("asset-a"), fail_if_staged)
        .await
        .expect("replay");
    assert_eq!(receipt.get(), replay.get());
    assert_eq!(
        service
            .submit_photo_message(&p, "photo-request", send("asset-b"), fail_if_staged)
            .await
            .expect_err("changed photo")
            .code,
        Code::IdReused
    );
    assert_eq!(queries_reaching_native(&f), 0);
}

#[tokio::test]
async fn generic_replica_reads_require_system_read() {
    let f = fixture();
    f.store
        .state
        .with(|state| state.set_replica_status(json!({"role":"replica"})))
        .expect("replica status");
    let service = service(&f);
    let reader = principal("agent:reader", "agent", "read", &["system:read"]);
    for (operation, input) in [
        (
            "records.get",
            r#"{"collection":"system.replica","id":"local"}"#,
        ),
        ("records.query", r#"{"collection":"system.replica"}"#),
        (
            "search",
            r#"{"collection":"system.replica","text":"replica"}"#,
        ),
    ] {
        for grants in [&["notes:write"][..], &["notes:read", "notes:write"]] {
            let writer = principal("agent:writer", "agent", "full", grants);
            let error = service
                .invoke(&writer, operation, input.as_bytes())
                .await
                .expect_err("system read grant required");
            assert_eq!(error.code, Code::Forbidden);
            assert_eq!(error.message, "missing required scope system:read");
        }
        let result = service
            .invoke(&reader, operation, input.as_bytes())
            .await
            .expect("system read grant");
        let result: Value = serde_json::from_str(result.get()).expect("result");
        assert_eq!(result["records"][0]["id"], "local");
    }
}

#[derive(Debug)]
struct ListClock(StdMutex<jiff::Timestamp>);

impl crate::clock::Clock for ListClock {
    fn now(&self) -> jiff::Timestamp {
        *self.0.lock().expect("clock")
    }
}

#[tokio::test]
async fn command_list_limits_visible_results_in_descending_creation_order() {
    let mut f = fixture();
    let start: jiff::Timestamp = "2026-11-20T08:00:00Z".parse().expect("clock");
    let clock = Arc::new(ListClock(StdMutex::new(start)));
    Arc::get_mut(&mut f.gateway).expect("gateway").clock = clock.clone();
    let p = agent();
    let other = principal("agent:other", "agent", "approve", &["messages:write"]);
    let mut expected = Vec::new();
    for (id, micros, principal) in [
        ("z-oldest", 0, &p),
        ("a-newest-visible", 1, &p),
        ("other-newest", 2, &other),
    ] {
        *clock.0.lock().expect("clock") = start + jiff::SignedDuration::from_micros(micros);
        let receipt = f
            .gateway
            .submit(
                principal,
                command(
                    id,
                    "messages.send",
                    &json!({"chat_id":"chat_synthetic","text":"synthetic"}),
                ),
            )
            .await
            .expect("admit");
        if principal.id == p.id {
            let record: CommandRecord = serde_json::from_str(receipt.get()).expect("record");
            expected.push(record.command.id);
        }
    }
    let service = service(&f);
    let list = service
        .invoke(&p, "commands.list", br#"{"limit":1}"#)
        .await
        .expect("list");
    let list: Value = serde_json::from_str(list.get()).expect("list body");
    assert_eq!(list.as_array().expect("rows").len(), 1);
    assert_eq!(list[0]["command"]["command_id"], expected[1]);
    let next = service
        .invoke(&p, "commands.list", br#"{"limit":1,"offset":1}"#)
        .await
        .expect("second page");
    let next: Value = serde_json::from_str(next.get()).expect("page body");
    assert_eq!(next.as_array().expect("rows").len(), 1);
    assert_eq!(next[0]["command"]["command_id"], expected[0]);
}

#[tokio::test]
async fn explicit_source_reads_with_missing_grants_fail_before_returning_data() {
    let f = fixture();
    let service = service(&f);
    let without_read = principal(
        "agent:without-read",
        "agent",
        "full",
        &["notes:write", "reminders:write"],
    );
    for (name, scope, input) in [
        ("notes.list", "notes:read", "{}"),
        (
            "reminders.items",
            "reminders:read",
            r#"{"smart":"all","time_zone":"UTC"}"#,
        ),
    ] {
        let error = service
            .invoke(&without_read, name, input.as_bytes())
            .await
            .expect_err("read forbidden");
        assert_eq!(error.code, Code::Forbidden);
        assert_eq!(error.message, format!("missing required scope {scope}"));
        assert!(!error.retryable);
        let allowed = principal("agent:allowed", "agent", "full", &[scope]);
        let result = service
            .invoke(&allowed, name, input.as_bytes())
            .await
            .expect("authorized empty read");
        let value: Value = serde_json::from_str(result.get()).expect("body");
        assert_eq!(value["count"], 0);
    }
}

#[tokio::test]
async fn reminder_title_replay_keeps_its_admitted_target_after_a_restart() {
    let f = fixture();
    let p = principal(
        "agent:reminder",
        "agent",
        "approve",
        &["reminders:read", "reminders:write"],
    );
    let commit = |checkpoint: &str, predecessor: &str, key: &str, title: &str| {
        let capture = serde_json::from_value(json!({
            "source":"reminders",
            "generation":"synthetic",
            "checkpoint":checkpoint,
            "predecessor":predecessor,
            "observations":[{
                "kind":"upsert",
                "collection":"reminders.reminder",
                "native_key":key,
                "observed_at":"2026-11-20T06:00:00Z",
                "data":{"title":title,"completed":false}
            }]
        }))
        .expect("capture");
        f.store.state.commit(&capture).expect("commit");
    };
    commit("first", "", "original", "Buy tickets");
    let request = command(
        "synthetic-reminder",
        "reminders.complete",
        &json!({"reminder":"Buy tickets"}),
    );
    let reply = f
        .gateway
        .submit(&p, request.clone())
        .await
        .expect("admit title");
    let original = f
        .gateway
        .state
        .principal_command(&p.id, &request.id)
        .expect("admitted command");
    let target: Value = serde_json::from_str(original.command.args.get()).expect("target");
    assert!(target["id"].is_string());
    assert!(original.command.expected_version > 0);
    commit("second", "first", "original", "Renamed");
    commit("third", "second", "replacement", "Buy tickets");
    let competing = f
        .store
        .state
        .0
        .lock()
        .expect("state")
        .reminder_target("Buy tickets", false)
        .expect("new target");
    assert_ne!(target["id"], competing.id.as_str());
    let Fixture {
        home,
        gateway,
        peer,
        blobs,
        store,
    } = f;
    drop((gateway, store, blobs));
    let reopened = Arc::new(LocalGateway::new(
        Arc::new(GatewayStore::open(home.path()).expect("reopen")),
        Arc::new(TestIdentity(p.clone())),
        Arc::new(TestCodec),
        peer,
        Arc::new(NativeActions),
    ));
    assert_eq!(
        reopened
            .submit(&p, request)
            .await
            .expect("bound replay")
            .get(),
        reply.get()
    );
    let changed = command(
        "synthetic-reminder",
        "reminders.complete",
        &json!({"reminder":"Renamed"}),
    );
    assert_eq!(
        reopened
            .submit(&p, changed)
            .await
            .expect_err("different selector")
            .code,
        Code::IdReused
    );
}

#[tokio::test]
async fn calendar_composition_and_projection_use_visible_state() {
    let f = fixture();
    let p = principal("agent:calendar", "agent", "read", &["calendar:read"]);
    f.store
        .state
        .commit(
            &serde_json::from_value(json!({
                "source":"calendar",
                "generation":"synthetic",
                "checkpoint":"1",
                "observations":[{
                    "kind":"upsert",
                    "collection":"calendar.event",
                    "native_key":"synthetic-event",
                    "observed_at":"2026-11-20T06:00:00Z",
                    "data":{
                        "title":"Synthetic calendar event",
                        "notes":"Synthetic body",
                        "start":"2026-11-20T10:00:00Z",
                        "end":"2026-11-20T11:00:00Z",
                        "all_day":false
                    }
                }]
            }))
            .expect("capture"),
        )
        .expect("commit");
    let service = service(&f);
    let agenda = service
        .invoke(
            &p,
            "calendar.agenda",
            br#"{
        "start":"2026-11-20T09:00:00Z","end":"2026-11-20T12:00:00Z","fields":["title"]
    }"#,
        )
        .await
        .expect("agenda");
    let agenda: Value = serde_json::from_str(agenda.get()).expect("agenda response");
    assert_eq!(
        agenda["records"][0]["data"],
        json!({"title":"Synthetic calendar event"})
    );
    assert!(agenda["records"][0]["id"].is_string());
    assert!(agenda["cursor"].is_object());
    let slots = service
        .calendar_free_slots(
            &p,
            &crate::composite_reads::FreeSlotsQuery {
                start: "2026-11-20T09:00:00Z".parse().expect("start"),
                end: "2026-11-20T12:00:00Z".parse().expect("end"),
                zone: jiff::tz::TimeZone::UTC,
                day_start: "09:00".parse().expect("opens"),
                day_end: "12:00".parse().expect("closes"),
                weekends: false,
                minutes: 30,
                limit: 10,
                calendar: None,
            },
        )
        .expect("free slots");
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].start, "2026-11-20T09:00:00Z");
    assert_eq!(slots[0].end, "2026-11-20T10:00:00Z");
    assert_eq!(slots[1].start, "2026-11-20T11:00:00Z");
    assert_eq!(slots[1].end, "2026-11-20T12:00:00Z");
}

#[tokio::test]
async fn ambiguous_command_arguments_are_rejected_before_replay_lookup() {
    let f = fixture();
    let p = principal("agent:focus", "agent", "approve", &["focus:write"]);
    let mut request = command(
        "synthetic-focus",
        "focus.set",
        &json!({"on":true,"mode":"Do Not Disturb"}),
    );
    f.gateway
        .submit(&p, request.clone())
        .await
        .expect("unambiguous command");
    for args in [
        r#"{"on":true,"ON":false,"mode":"Do Not Disturb"}"#,
        r#"{"ON":false,"on":true,"mode":"Do Not Disturb"}"#,
        r#"{"on":false,"on":true,"mode":"Do Not Disturb"}"#,
    ] {
        request.args = serde_json::from_str(args).expect("raw arguments");
        assert_eq!(
            f.gateway
                .submit(&p, request.clone())
                .await
                .expect_err("ambiguous arguments")
                .code,
            Code::Invalid
        );
    }
    assert_eq!(f.gateway.state.commands().expect("commands").len(), 1);
}

#[tokio::test]
async fn the_same_caller_command_id_is_independent_for_each_principal() {
    let f = fixture();
    let first = agent();
    let second = principal("agent:other", "agent", "approve", &["messages:write"]);
    let request = |text: &str| {
        command(
            "shared-request",
            "messages.send",
            &json!({"chat_id":"chat_synthetic","text":text}),
        )
    };
    let first_receipt = f
        .gateway
        .submit(&first, request("first"))
        .await
        .expect("first admission");
    let service = service(&f);
    assert_eq!(
        service
            .request_status(&second, "shared-request", false)
            .await
            .expect_err("no command for this principal")
            .code,
        Code::NotFound
    );
    let second_receipt = f
        .gateway
        .submit(&second, request("second"))
        .await
        .expect("second admission");
    let first_record: CommandRecord =
        serde_json::from_str(first_receipt.get()).expect("first record");
    let second_record: CommandRecord =
        serde_json::from_str(second_receipt.get()).expect("second record");
    assert_ne!(first_record.command.id, second_record.command.id);
    assert_eq!(first_record.principal, first.id);
    assert_eq!(second_record.principal, second.id);
    for (principal, text, receipt) in [
        (&first, "first", first_receipt),
        (&second, "second", second_receipt),
    ] {
        assert_eq!(
            service
                .request_status(principal, "shared-request", false)
                .await
                .expect("own status")
                .get(),
            receipt.get()
        );
        assert_eq!(
            f.gateway
                .submit(principal, request(text))
                .await
                .expect("replay")
                .get(),
            receipt.get()
        );
        assert_eq!(
            f.gateway
                .submit(principal, request("changed"))
                .await
                .expect_err("rebound ID")
                .code,
            Code::IdReused
        );
    }
}

#[tokio::test]
async fn generic_reads_require_read_grants_even_with_write_access() {
    let f = fixture();
    let service = service(&f);
    let cursor = service.store.head().expect("cursor before capture");
    let blob = b"synthetic note blob";
    let (digest, _) = f.blobs.lock().await.stage(blob).expect("staged blob");
    f.store
        .state
        .commit(
            &serde_json::from_value(json!({
                "source":"notes", "generation":"synthetic", "checkpoint":"1",
                "observations":[{
                    "kind":"upsert", "collection":"notes.note", "native_key":"fixture-note",
                    "observed_at":"2026-11-20T06:00:00Z", "data":{
                        "title":"Synthetic fixture","raw_body_digest":digest.as_str(),"size":blob.len()
                    }
                }]
            }))
            .expect("capture"),
        )
        .expect("commit");
    let write_only = principal("agent:writer", "agent", "full", &["notes:write"]);
    let reader = principal("agent:reader", "agent", "read", &["notes:read"]);
    let mixed = principal(
        "agent:mixed",
        "agent",
        "full",
        &["notes:write", "calendar:read"],
    );
    let query = br#"{"collection":"notes.note"}"#;
    let notes = service
        .invoke(&reader, "records.query", query)
        .await
        .expect("read notes");
    let notes: Value = serde_json::from_str(notes.get()).expect("notes");
    let id = notes["records"][0]["id"].as_str().expect("note ID");
    let lookup = json!({"collection":"notes.note","id":id}).to_string();
    let search = br#"{"text":"Synthetic"}"#;
    for (operation, input) in [
        ("records.get", lookup.as_bytes()),
        ("records.query", &query[..]),
        ("search", &search[..]),
    ] {
        assert_eq!(
            service
                .invoke(&write_only, operation, input)
                .await
                .expect_err("write-only credential")
                .code,
            Code::Forbidden
        );
        let result = service
            .invoke(&reader, operation, input)
            .await
            .expect("read credential");
        let result: Value = serde_json::from_str(result.get()).expect("records");
        assert_eq!(result["records"].as_array().expect("list").len(), 1);
    }
    assert_eq!(
        service
            .invoke(&mixed, "records.get", lookup.as_bytes())
            .await
            .expect_err("wrong source read grant")
            .code,
        Code::Forbidden
    );
    let result = service
        .invoke(&mixed, "search", search)
        .await
        .expect("search across readable sources");
    let result: Value = serde_json::from_str(result.get()).expect("records");
    assert_eq!(result["records"], json!([]));
    assert_eq!(
        service
            .store
            .blob(&mixed.visibility(), id, digest.as_str())
            .await
            .expect_err("blob denied")
            .code,
        Code::Unauthorized,
    );
    assert_eq!(
        service
            .store
            .blob(&reader.visibility(), id, digest.as_str())
            .await
            .expect("read blob"),
        blob,
    );
    for (principal, visible) in [(&mixed, false), (&reader, true)] {
        let batch = service
            .store
            .changes(&principal.visibility(), &cursor)
            .expect("changes")
            .expect("batch");
        assert_eq!(batch.get().contains(id), visible);
    }
}

#[tokio::test]
async fn a_combining_mark_only_reminder_selector_cannot_match_the_only_reminder() {
    let f = fixture();
    let p = principal(
        "agent:reminder",
        "agent",
        "approve",
        &["reminders:read", "reminders:write"],
    );
    f.store.state.commit(&serde_json::from_value(json!({
        "source":"reminders", "generation":"synthetic", "checkpoint":"1",
        "observations":[{
            "kind":"upsert", "collection":"reminders.reminder", "native_key":"fixture-reminder",
            "observed_at":"2026-11-20T06:00:00Z", "data":{"title":"Unrelated reminder","completed":false}
        }]
    })).expect("capture")).expect("commit");
    let request = command(
        "empty-selector",
        "reminders.complete",
        &json!({"reminder":"\u{0301}"}),
    );
    assert_eq!(
        f.gateway
            .submit(&p, request)
            .await
            .expect_err("empty normalized selector")
            .code,
        Code::Invalid
    );
    assert!(f.gateway.state.commands().expect("commands").is_empty());
}
