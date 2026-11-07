use super::*;

fn mutations() -> Vec<(&'static str, Value)> {
    vec![
        (
            "photos.favorite",
            json!({"asset_id":"synthetic-asset","favorite":true}),
        ),
        ("photos.album_create", json!({"title":"synthetic-album"})),
        (
            "photos.album_rename",
            json!({"album_id":"synthetic-album-id","title":"synthetic-title"}),
        ),
        (
            "photos.album_add",
            json!({"album_id":"synthetic-album-id","asset_ids":["synthetic-asset"]}),
        ),
        (
            "photos.album_remove",
            json!({"album_id":"synthetic-album-id","asset_ids":["synthetic-asset"]}),
        ),
        (
            "photos.album_delete",
            json!({"album_id":"synthetic-album-id"}),
        ),
    ]
}

#[tokio::test]
async fn photos_writes_require_caller_ids_before_local_or_remote_dispatch() {
    let f = fixture();
    let p = principal("agent:photos", "agent", "full", &["photos:write"]);
    for (operation, args) in mutations() {
        let descriptor = ab_catalog::operations()
            .expect("catalog")
            .into_iter()
            .find(|descriptor| descriptor.name == operation)
            .expect("operation");
        assert!(
            descriptor.input["required"]
                .as_array()
                .expect("required")
                .contains(&json!("id"))
        );
        let mut request = query(operation, &args);
        request.id.clear();
        for caller in [owner(), p.clone()] {
            assert_eq!(
                f.gateway
                    .query(&caller, request.clone())
                    .await
                    .expect_err("explicit ID required")
                    .code,
                Code::Invalid
            );
        }
    }
    let delegated = principal(
        "replica:fixture/agent:photos",
        "delegated",
        "full",
        &["photos:write"],
    );
    let service = crate::Service {
        store: Arc::new(LocationReads),
        identity: Arc::new(TestIdentity(p.clone())),
        codec: Arc::new(TestCodec),
        target: crate::CommandTarget::Remote(Arc::new(crate::RemoteGateway {
            route: crate::GatewayRoute::Feed,
            transport: Arc::new(LocationForward {
                gateway: f.gateway.clone(),
                principal: delegated,
            }),
            identity: Arc::new(TestIdentity(p.clone())),
            store: Arc::new(LocationReads),
            codec: Arc::new(TestCodec),
        })),
    };
    for (operation, args) in mutations() {
        let input = serde_json::to_vec(&json!({"args":args})).expect("request");
        assert_eq!(
            service
                .invoke(&p, operation, &input)
                .await
                .expect_err("forwarding cannot mint a mutation ID")
                .code,
            Code::Invalid
        );
    }
    assert!(f.gateway.state.commands().expect("ledger").is_empty());
    assert_eq!(queries_reaching_native(&f), 0);
}

#[tokio::test]
async fn every_photos_write_requires_approval_and_never_persists_library_details() {
    let declared: std::collections::BTreeSet<_> = ab_catalog::operations()
        .expect("catalog")
        .into_iter()
        .filter(|operation| operation.scope == "photos:write")
        .map(|operation| operation.name)
        .collect();
    let exercised: std::collections::BTreeSet<_> = mutations()
        .into_iter()
        .map(|(name, _)| name.to_owned())
        .collect();
    assert_eq!(declared, exercised);
    for level in ["approve", "full"] {
        for (operation, args) in mutations() {
            let f = fixture();
            let p = principal("agent:photos", "agent", level, &["photos:write"]);
            let mut request = query(operation, &args);
            request.id = "qry_photos_once".into();
            let pending = f.gateway.query(&p, request.clone()).await.expect("pending");
            assert_eq!(state_of(&pending), "awaiting_approval");
            assert_eq!(queries_reaching_native(&f), 0);
            let stored = serde_json::to_string(&f.gateway.state.commands().expect("ledger"))
                .expect("ledger JSON");
            for secret in ["synthetic-asset", "synthetic-album", "synthetic-title"] {
                assert!(!stored.contains(secret), "{operation}");
            }
            assert!(stored.contains("request_digest"));
            let previews = f.gateway.approvals().expect("owner previews");
            assert_eq!(
                serde_json::from_str::<Value>(previews[0].command.args.get()).expect("preview"),
                args
            );
            let mut changed = request.clone();
            changed.args = serde_json::value::to_raw_value(&match operation {
                "photos.favorite" => json!({"asset_id":"different","favorite":true}),
                "photos.album_create" => json!({"title":"different"}),
                "photos.album_rename" => json!({"album_id":"different","title":"synthetic-title"}),
                "photos.album_add" | "photos.album_remove" => {
                    json!({"album_id":"different","asset_ids":["synthetic-asset"]})
                }
                _ => json!({"album_id":"different"}),
            })
            .expect("args");
            assert_eq!(
                f.gateway.query(&p, changed).await.expect_err("bound").code,
                Code::IdReused
            );
            f.gateway
                .decide(
                    &owner(),
                    &request.id,
                    true,
                    &OwnerReview {
                        binding: "",
                        accepted_unpreviewable: false,
                    },
                )
                .await
                .expect("approved");
            assert_eq!(queries_reaching_native(&f), 0);
            let (first, duplicate) = tokio::join!(
                f.gateway.query(&p, request.clone()),
                f.gateway.query(&p, request.clone())
            );
            assert_ne!(first.is_ok(), duplicate.is_ok());
            assert_eq!(queries_reaching_native(&f), 1);
            assert!(f.gateway.query(&p, request).await.is_err());
            let rows = f.gateway.state.commands().expect("ledger");
            assert_eq!(rows[0].state, ab_trust::domain::CommandState::Observed);
            assert!(
                !serde_json::to_string(&rows)
                    .expect("JSON")
                    .contains("synthetic-asset")
            );
        }
    }
}

#[derive(Debug)]
struct PhotoBlobReply;
impl crate::GatewayTransport for PhotoBlobReply {
    fn connected(&self) -> bool {
        true
    }
    fn exchange<'a>(
        &'a self,
        _: &'a crate::GatewayRoute,
        _: String,
        _: Vec<u8>,
    ) -> crate::Task<'a, crate::ForwardReply> {
        Box::pin(async move {
            Ok(crate::ForwardReply {
                result: serde_json::value::to_raw_value(&json!({"result_json":{},"blobs":[{"digest":"00".repeat(32),"size":1,"handle":"synthetic"}]})).map_err(|_| Code::Corrupt)?,
                lookup_only: false,
            })
        })
    }
}

#[tokio::test]
async fn photos_never_hydrates_blobs_from_native_or_forwarded_replies() {
    let f = fixture();
    let p = principal("agent:photos", "agent", "read", &["photos:read"]);
    let mut request = query("photos.get", &json!({"asset_id":"synthetic"}));
    request.id = "qry_photos_blobs".into();
    assert_eq!(
        f.gateway
            .query(&p, request)
            .await
            .expect_err("native blobs forbidden")
            .code,
        Code::Corrupt
    );
    assert_eq!(f.blobs.lock().await.used(), 0);
    let service = crate::Service {
        store: Arc::new(LocationReads),
        identity: Arc::new(TestIdentity(p.clone())),
        codec: Arc::new(TestCodec),
        target: crate::CommandTarget::Remote(Arc::new(crate::RemoteGateway {
            route: crate::GatewayRoute::Feed,
            transport: Arc::new(PhotoBlobReply),
            identity: Arc::new(TestIdentity(p.clone())),
            store: Arc::new(LocationReads),
            codec: Arc::new(TestCodec),
        })),
    };
    assert_eq!(
        service
            .invoke(&p, "photos.get", br#"{"args":{"asset_id":"synthetic"}}"#)
            .await
            .expect_err("forwarded blobs forbidden")
            .code,
        Code::Corrupt
    );
}

#[tokio::test]
async fn photos_scopes_and_read_levels_fail_before_native_dispatch() {
    let f = fixture();
    for p in [
        principal("agent:read", "agent", "read", &["photos:write"]),
        principal("agent:other", "agent", "full", &["location:write"]),
    ] {
        for (operation, args) in mutations() {
            assert_eq!(
                f.gateway
                    .query(&p, query(operation, &args))
                    .await
                    .expect_err("forbidden")
                    .code,
                Code::Forbidden
            );
        }
    }
    assert_eq!(queries_reaching_native(&f), 0);
    assert!(f.gateway.state.commands().expect("ledger").is_empty());
}

#[tokio::test]
async fn photos_native_failure_is_indeterminate_and_cannot_redispatch() {
    let f = fixture();
    let mut request = query(
        "photos.favorite",
        &json!({"asset_id":"synthetic-asset","favorite":true}),
    );
    request.id = "qry_photos_failure".into();
    assert_eq!(
        f.gateway
            .query(&owner(), request.clone())
            .await
            .expect_err("failed")
            .code,
        Code::Indeterminate
    );
    assert!(f.gateway.query(&owner(), request).await.is_err());
    assert_eq!(queries_reaching_native(&f), 1);
    assert_eq!(
        f.gateway.state.commands().expect("ledger")[0].state,
        ab_trust::domain::CommandState::Indeterminate
    );
}

#[tokio::test]
async fn photos_reads_are_transient_and_transfer_identity_cannot_be_spoofed() {
    let f = fixture();
    let p = principal("agent:photos", "agent", "read", &["photos:read"]);
    for (operation, args) in [
        ("photos.search", json!({"limit":200})),
        ("photos.get", json!({"asset_id":"synthetic-asset"})),
        ("photos.albums", json!({"smart":true})),
        (
            "photos.fetch",
            json!({"asset_id":"synthetic-asset","resource_index":1,"network_access":true,"max_bytes":68_719_476_736_u64}),
        ),
        (
            "photos.fetch",
            json!({"transfer_id":"synthetic-transfer","offset":68_719_476_736_u64,"length":524_288}),
        ),
        (
            "photos.release",
            json!({"transfer_id":"synthetic-transfer"}),
        ),
    ] {
        f.gateway
            .query(&p, query(operation, &args))
            .await
            .expect("native read");
    }
    {
        let frames = f.peer.0.frames.lock().expect("frames");
        assert_eq!(frames.len(), 6);
        assert!(
            frames
                .iter()
                .all(|args| args["_principal"] == "agent:photos")
        );
        assert!(frames.iter().all(|args| {
            args["_request"]
                .as_str()
                .is_some_and(|id| id.starts_with("qry_"))
        }));
    }
    assert!(f.gateway.state.commands().expect("ledger").is_empty());
    assert_eq!(
        f.store
            .state
            .with(|state| state.head())
            .expect("head")
            .sequence,
        0
    );
    for (operation, args) in [
        ("photos.search", json!({"_principal":"foreign"})),
        (
            "photos.fetch",
            json!({"asset_id":"synthetic","_request":"foreign"}),
        ),
        ("photos.search", json!({"limit":201})),
        ("photos.search", json!({"near":{"lat":91,"lon":0}})),
        ("photos.search", json!({"start":"not-a-date"})),
        (
            "photos.search",
            json!({"start":"2020-01-02T00:00:00Z","end":"2020-01-01T00:00:00Z"}),
        ),
        ("photos.search", json!({"radius_m":1})),
        ("photos.get", json!({})),
        ("photos.fetch", json!({})),
        (
            "photos.fetch",
            json!({"transfer_id":"synthetic","asset_id":"synthetic"}),
        ),
        (
            "photos.fetch",
            json!({"transfer_id":"synthetic","offset":-1}),
        ),
        (
            "photos.fetch",
            json!({"transfer_id":"synthetic","length":524_289}),
        ),
        (
            "photos.fetch",
            json!({"asset_id":"synthetic","resource_key":"short"}),
        ),
        (
            "photos.fetch",
            json!({"asset_id":"synthetic","max_bytes":68_719_476_737_u64}),
        ),
        ("photos.release", json!({"transfer_id":null})),
    ] {
        assert_eq!(
            f.gateway
                .query(&p, query(operation, &args))
                .await
                .expect_err("invalid")
                .code,
            Code::Invalid
        );
    }
    assert_eq!(queries_reaching_native(&f), 6);
}

#[tokio::test]
async fn photos_approval_requires_a_live_exact_preview_and_browser_decision() {
    let f = fixture();
    let review = OwnerReview {
        binding: "",
        accepted_unpreviewable: false,
    };
    let p = principal("agent:photos", "agent", "full", &["photos:write"]);
    let mut request = query(
        "photos.album_create",
        &json!({"title":"synthetic-private-title"}),
    );
    request.id = "qry_photos_preview".into();
    f.gateway.query(&p, request.clone()).await.expect("pending");
    let stored = f.gateway.state.command(&request.id).expect("record");
    assert_eq!(
        f.gateway
            .decide_phone(&request.id, &stored.request_hash, true)
            .await
            .expect_err("browser only")
            .code,
        Code::Unsupported
    );
    f.gateway
        .photos_previews
        .remove(&request.id)
        .expect("simulate restart");
    assert!(f.gateway.approval_review(&stored).await.is_err());
    assert_eq!(
        f.gateway
            .decide(&owner(), &request.id, true, &review)
            .await
            .expect_err("preview unavailable")
            .code,
        Code::Unavailable
    );
    let preview = f.gateway.approvals().expect("hash-only fallback");
    assert!(
        !preview[0]
            .command
            .args
            .get()
            .contains("synthetic-private-title")
    );
    assert_eq!(
        state_of(
            &f.gateway
                .query(&p, request.clone())
                .await
                .expect("restore preview")
        ),
        "awaiting_approval"
    );
    let preview = f.gateway.approvals().expect("restored");
    assert!(
        preview[0]
            .command
            .args
            .get()
            .contains("synthetic-private-title")
    );
    f.gateway
        .approval_review(&preview[0])
        .await
        .expect("review exact args");
    f.gateway
        .photos_previews
        .remember(
            &request.id,
            &serde_json::value::to_raw_value(&json!({"title":"different"})).expect("args"),
        )
        .expect("simulate corrupt preview");
    assert_eq!(
        f.gateway
            .decide(&owner(), &request.id, true, &review)
            .await
            .expect_err("digest mismatch")
            .code,
        Code::Corrupt
    );
    f.gateway
        .decide(&owner(), &request.id, false, &review)
        .await
        .expect("deny without private preview");
    assert!(f.gateway.query(&p, request).await.is_err());
    assert_eq!(queries_reaching_native(&f), 0);
    assert_eq!(
        f.gateway.state.commands().expect("ledger")[0].state,
        ab_trust::domain::CommandState::RejectedByOwner
    );
}

#[tokio::test]
async fn delegated_photos_write_uses_the_same_owner_gate() {
    let f = fixture();
    let p = principal("agent:photos", "agent", "full", &["photos:write"]);
    let delegated = principal(
        "replica:fixture/agent:photos",
        "delegated",
        "full",
        &["photos:write"],
    );
    let service = crate::Service {
        store: Arc::new(LocationReads),
        identity: Arc::new(TestIdentity(p.clone())),
        codec: Arc::new(TestCodec),
        target: crate::CommandTarget::Remote(Arc::new(crate::RemoteGateway {
            route: crate::GatewayRoute::Feed,
            transport: Arc::new(LocationForward {
                gateway: f.gateway.clone(),
                principal: delegated,
            }),
            identity: Arc::new(TestIdentity(p.clone())),
            store: Arc::new(LocationReads),
            codec: Arc::new(TestCodec),
        })),
    };
    let input = br#"{"id":"qry_forwarded_photos","args":{"asset_id":"synthetic-asset","favorite":true},"deadline_ms":5000}"#;
    assert_eq!(
        state_of(
            &service
                .invoke(&p, "photos.favorite", input)
                .await
                .expect("pending")
        ),
        "awaiting_approval"
    );
    f.gateway
        .decide(
            &owner(),
            "qry_forwarded_photos",
            true,
            &OwnerReview {
                binding: "",
                accepted_unpreviewable: false,
            },
        )
        .await
        .expect("approved");
    service
        .invoke(&p, "photos.favorite", input)
        .await
        .expect("once");
    assert!(service.invoke(&p, "photos.favorite", input).await.is_err());
    assert_eq!(queries_reaching_native(&f), 1);
}
