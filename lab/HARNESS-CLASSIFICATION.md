# Harness classification

Baseline: Go main 10a397d6; the added passkey golden test has no internal imports. impl=go. This inventories direct internal imports, including tests and build-tagged files. Categories describe coupling, not a passing Rust qualification. Scenario timing and execution coverage belong to the separate TEST-INVENTORY.md worker.

Precedence: D-class home initialization/migration or writes; P-class product execution; read-only inspection; wire-contract only. InspectHome is explicitly read-only (SQLite mode=ro); unlike Open, it must not be treated as a migration. No lab file in this baseline imports InspectHome directly. P includes HTTP handlers, semantic validators, decoders and simulated peers using product implementations. Wire classification does not assert oracle independence. Files may contain several kinds; the highest-risk kind wins. Transitive calls must be audited per journey before Rust evidence is accepted. Contact-handle normalization, blob reference extraction, command evidence interpretation and grant selection count as P-class even when exported by contracts. Wire shape/schema validation and identifier construction alone stay wire-contract only.

The table is a conservative source inventory. Every P/D journey must drive the selected product binary before it qualifies a Rust module. Pure read-only SQLite/HTTP probes without internal imports are outside this table.

| Category | Files |
| --- | ---: |
| wire-contract only | 181 |
| read-only inspection | 4 |
| P-class | 71 |
| D-class | 77 |

Total: 333 files. Existing evidence annotations record the implementation that produced the original results; they do not refresh timestamps or change verdicts. All 746 historical evidence JSON files retain their previous data apart from the added impl field.

| File | Classification | Basis | Internal imports |
| --- | --- | --- | --- |
| `lab/agent/mail.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/agent/messages.go` | P-class | Product semantic helper in-process: NormalizeContactHandle | contracts |
| `lab/agent/query.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/agent/runner.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/agent/usage.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/demo/dataset.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/demo/fixtures.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/demo/world.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/iphonegateway/main.go` | D-class | Home constructor/inspection may initialize or migrate: Open | blobs, iphone, state, contracts, frontdoor, operations, trust |
| `lab/relayprobe/agents.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/relayprobe/commands.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/relayprobe/contacts.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/relayprobe/contacts_watch.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/relayprobe/live.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/relayprobe/postburst.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/relayprobe/query.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/relayprobe/replay.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust |
| `lab/relayprobe/revocation.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/relayprobe/transport.go` | P-class | Product implementation linked in-process: trust | trust |
| `lab/scenarios/actions.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/agent_access.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/agent_activity.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/agent_activity_capture.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/agent_capture_rejection.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/agent_credentials.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/agent_level_boundaries.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/agent_levels.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/agent_oauth_cimd.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web |
| `lab/scenarios/agent_oauth_credentials.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | contracts, trust |
| `lab/scenarios/agent_oauth_grants.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/agent_oauth_http.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust |
| `lab/scenarios/audio_queries.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/auth_scopes_metadata.go` | P-class | Product semantic helper in-process: GrantsForSource | contracts |
| `lab/scenarios/automatic.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/blob_journey.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | blobs, contracts |
| `lab/scenarios/blob_native.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_effect.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_invalidation.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_large.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_live_delta.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_stamp.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_store.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_store_perf.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_store_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/calendar_verification.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/capture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/capture_observer_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/capture_observers.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/clock_pack.go` | P-class | Product implementation linked in-process: clockpack, shortcut | clockpack, contracts, shortcut |
| `lab/scenarios/clock_real.go` | P-class | Product implementation linked in-process: clockpack, shortcut | clockpack, shortcut |
| `lab/scenarios/clock_session_test.go` | P-class | Product implementation linked in-process: clockpack, shortcut | clockpack, shortcut |
| `lab/scenarios/clock_simulation.go` | P-class | Product implementation linked in-process: clockpack, shortcut | clockpack, shortcut |
| `lab/scenarios/clock_timer.go` | P-class | Product implementation linked in-process: clockpack | clockpack |
| `lab/scenarios/clock_timer_native.go` | P-class | Product implementation linked in-process: shortcut | shortcut |
| `lab/scenarios/commands.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/connector_approval.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/connector_real.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, remote, trust, web |
| `lab/scenarios/consent.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/consent_activity.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, state |
| `lab/scenarios/contact_recipient_lookup.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/contacts.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/contacts_groups.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/contacts_identity.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/contacts_list.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/contacts_relay.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/date_fixtures.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/delegation.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/demo_item_coverage.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/demo_media_coverage.go` | P-class | Product semantic helper in-process: BlobReferences | contracts |
| `lab/scenarios/demo_poster_quality.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/demo_readmodel_coverage.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/demo_rich.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/files.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/files_commands.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/files_replica.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/focus_pack.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, focuspack, shortcut, trust |
| `lab/scenarios/focus_real.go` | P-class | Product implementation linked in-process: shortcut | shortcut |
| `lab/scenarios/gateway_dials.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/helper_ipc.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/scenarios/helper_ipc_commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/scenarios/helper_release.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/integrity.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/iphone_capability.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, state, trust |
| `lab/scenarios/iphone_capture.go` | D-class | Home constructor/inspection may initialize or migrate: Open | blobs, contracts, iphone, operations, state, trust |
| `lab/scenarios/iphone_command.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, relay, state, trust |
| `lab/scenarios/iphone_connector_real.go` | P-class | Product implementation linked in-process: iphone, trust, web | contracts, iphone, trust, web |
| `lab/scenarios/iphone_connector_real_test.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust |
| `lab/scenarios/iphone_health.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, state, trust |
| `lab/scenarios/iphone_health_write.go` | P-class | Product semantic helper in-process: ValidateIPhoneCommand | contracts |
| `lab/scenarios/iphone_live.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, relay, state, trust |
| `lab/scenarios/iphone_location.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, state, trust |
| `lab/scenarios/iphone_pair.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust |
| `lab/scenarios/iphone_real.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, iphone, operations, relay, state, trust |
| `lab/scenarios/iphone_real_test.go` | D-class | Home constructor/inspection may initialize or migrate: Open | iphone, trust |
| `lab/scenarios/iphone_screentime.go` | P-class | Product semantic helper in-process: ValidateIPhoneCommand | contracts |
| `lab/scenarios/iterator_pool.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mac.go` | P-class | Product semantic helper in-process: MacCompletionEvidence | contracts |
| `lab/scenarios/mac_actions.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/mac_clipboard.go` | P-class | Product semantic helper in-process: MacCompletionEvidence | contracts |
| `lab/scenarios/mac_commands.go` | P-class | Product semantic helper in-process: MacCompletionEvidence | contracts |
| `lab/scenarios/mac_extras.go` | P-class | Product semantic helper in-process: MacCompletionEvidence | contracts |
| `lab/scenarios/mac_extras_real.go` | P-class | Product semantic helper in-process: MacCompletionEvidence | contracts |
| `lab/scenarios/mac_notify_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mail_body_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mail_commands_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mail_read.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mail_replica.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/mail_runner.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/maps_queries.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/media.go` | read-only inspection | Direct SQLite inspection with mode=ro; no product constructor | contracts |
| `lab/scenarios/media_collection.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/media_export_real.go` | read-only inspection | Direct SQLite inspection with mode=ro; no product constructor | contracts |
| `lab/scenarios/media_export_safety.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/media_poster_native.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/media_source_geometry.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/media_yield.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/message_search.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/message_search_migration.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts |
| `lab/scenarios/messages_actions_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_chat_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_cleanup_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_commands.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_commands_real.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/messages_delete_visibility.go` | read-only inspection | Direct SQLite inspection with mode=ro; no product constructor | contracts |
| `lab/scenarios/messages_failed_ids.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_interactive.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/messages_media_cleanup.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_observed_ids.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_parts.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_parts_progress.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_resend_real.go` | P-class | Product semantic helper in-process: NormalizeContactHandle | contracts |
| `lab/scenarios/messages_rich.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_rich_handles.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/messages_run.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/native.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/native_app_cache.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/native_post_burst.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/native_session.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/native_visibility_test.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_capture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_capture_test.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_cleanup_ui.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_decode.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_external.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_extras.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_extras_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_list.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/notes_parity.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/permissions.go` | P-class | Product implementation linked in-process: runtime | contracts, runtime |
| `lab/scenarios/phone.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/queries.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/readmodel_handles.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodel_known_sender.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodel_participants.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts |
| `lab/scenarios/readmodel_perf.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts, state, trust |
| `lab/scenarios/readmodel_pin_preview.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodel_rich_attachments.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodel_rich_filters.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts |
| `lab/scenarios/readmodels_reminders.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_attachments.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_boundaries.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_fixtures.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_groups.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_hardening.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_items.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_items_boundaries.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_nesting.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_sorts.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/readmodels_reminders_zones.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/recordings.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_acme_pebble.go` | P-class | Product implementation linked in-process: relay, remote | relay, remote |
| `lab/scenarios/relay_agent_approvals.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/relay_agent_delegation.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/relay_agent_parent.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/relay_agents.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/relay_auth.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, replication, trust |
| `lab/scenarios/relay_auth_capture.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/relay_connect.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | contracts, remote, trust |
| `lab/scenarios/relay_dns.go` | P-class | Product implementation linked in-process: relay | relay |
| `lab/scenarios/relay_ingress.go` | P-class | Product implementation linked in-process: remote | remote |
| `lab/scenarios/relay_native.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_native_blobs.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_native_ready.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_pairing.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, replication, trust |
| `lab/scenarios/relay_postburst.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_public_agents.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/relay_real.go` | P-class | Product implementation linked in-process: remote | contracts, remote |
| `lab/scenarios/relay_security.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust |
| `lab/scenarios/relay_service.go` | D-class | Home constructor/inspection may initialize or migrate: Open | relay |
| `lab/scenarios/relay_snapshot.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/relay_streams.go` | P-class | Product implementation linked in-process: relay | relay |
| `lab/scenarios/reminders_demo.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/reminders_demo_readmodels.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/reminders_store.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/reminders_store_perf.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/reminders_store_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/remote.go` | P-class | Product implementation linked in-process: remote | contracts, remote |
| `lab/scenarios/remote_checks.go` | P-class | Product implementation linked in-process: remote | remote |
| `lab/scenarios/remote_flow.go` | P-class | Product implementation linked in-process: remote | remote |
| `lab/scenarios/remote_private.go` | D-class | Home constructor/inspection may initialize or migrate: Open | frontdoor, remote, trust |
| `lab/scenarios/remote_private_real.go` | P-class | Product implementation linked in-process: remote | remote |
| `lab/scenarios/replica_capture_atomic.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/replica_capture_staging.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts, state |
| `lab/scenarios/robust.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/runner.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/safari_capture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/safari_cleanup.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/safari_gateway.go` | P-class | Product semantic helper in-process: GrantsForSource | contracts |
| `lab/scenarios/scope.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/search_perf.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts, frontdoor, operations, state, trust |
| `lab/scenarios/security_failures.go` | P-class | Product implementation linked in-process: frontdoor, remote | frontdoor, remote |
| `lab/scenarios/security_oauth.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/security_owner.go` | P-class | Product implementation linked in-process: trust | contracts, trust |
| `lab/scenarios/security_relay.go` | D-class | Home constructor/inspection may initialize or migrate: Open | relay |
| `lab/scenarios/security_remote.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, remote, trust, web |
| `lab/scenarios/setup.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/shortcut_probe.go` | P-class | Product implementation linked in-process: shortcut | contracts, shortcut |
| `lab/scenarios/shortcut_probe_test.go` | P-class | Product implementation linked in-process: shortcut | shortcut |
| `lab/scenarios/shortcut_sign.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/shortcuts_runner.go` | P-class | Product implementation linked in-process: shortcut | contracts, shortcut |
| `lab/scenarios/sim_progress.go` | D-class | Direct writable product SQLite open or mutation; can create/alter a Rust home | contracts |
| `lab/scenarios/siri_intents.go` | P-class | Product implementation linked in-process: shortcut | shortcut |
| `lab/scenarios/snapshot_pool.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, state, trust |
| `lab/scenarios/source_grants.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, sourcefixture, state, trust, web |
| `lab/scenarios/source_support.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/sse.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/thread_originator.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/thread_originator_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/thread_originator_live.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/thread_originator_read.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/usage.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/usage_proof.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/vision_queries.go` | P-class | Product implementation linked in-process: runtime | contracts, runtime |
| `lab/scenarios/watch_snapshot.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_auth_isolation.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web |
| `lab/scenarios/web_avatar_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_browser_control.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_command_resolve.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/web_contacts_control.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_contacts_parity.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_cookie_boundaries.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web |
| `lab/scenarios/web_details_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_fixture_chunks.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_fixture_snapshot.go` | P-class | Product semantic helper in-process: BlobReferences | contracts |
| `lab/scenarios/web_handles_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_hosts.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web |
| `lab/scenarios/web_list_filter_oracle.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_list_filters_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_list_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_media_playback_scene.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_messages_read_real.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/web_parity_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_reaction_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_reminders_control.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_reminders_write_real.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_replica_session.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web |
| `lab/scenarios/web_rich_parts_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_search_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_server.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, web, contracts |
| `lab/scenarios/web_sources.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/web_sources_errors.go` | D-class | Home constructor/inspection may initialize or migrate: Open | operations, state, trust, web |
| `lab/scenarios/web_thread_overlay_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_transcript_effect_fixture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/web_upload_checkpoint.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_control_lab.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_control_production.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_feed.go` | P-class | Product implementation linked in-process: operations | contracts, operations |
| `lab/scenarios/web_upload_feed_forward.go` | P-class | Product implementation linked in-process: operations | contracts, operations |
| `lab/scenarios/web_upload_feed_latency.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/web_upload_feed_session.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_lease.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/web_upload_limits.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_native_corrupt.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/web_upload_native_real.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust, web |
| `lab/scenarios/web_upload_native_staging.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_probe.go` | D-class | Home constructor/inspection may initialize or migrate: Open | blobs, contracts, replication, state, trust |
| `lab/scenarios/web_upload_replica.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/scenarios/web_upload_replica_command.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_replica_diagnostic.go` | read-only inspection | Direct SQLite inspection with mode=ro; no product constructor | contracts |
| `lab/scenarios/web_upload_replica_forward_resume.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_replica_session.go` | P-class | Product implementation linked in-process: web | web |
| `lab/scenarios/web_upload_status_capture.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scenarios/web_upload_status_load.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/scripted/client.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/audio.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/blobs.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/calendar.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/capture.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/contacts.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/dataset.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/dataset_test.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/files.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/files_commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/focus.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/iphone.go` | P-class | Product implementation linked in-process: trust | trust |
| `lab/simapple/mac.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/mac_commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/mail.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/maps.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/maps_commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/messages.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/messages_commands.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/notes.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/pdf.go` | P-class | Product semantic helper in-process: BlobReferences | contracts |
| `lab/simapple/pdf_annotations.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/persistence.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/phone.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/playback.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/query.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/readmodels.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/recordings.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/reminders.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/routes.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/safari.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/shortcuts.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/simapple.go` | P-class | Product implementation linked in-process: applepeer | applepeer, contracts |
| `lab/simapple/speech.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/usage.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/simapple/vision.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/verify-media/main.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/world/credentials.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | contracts, trust |
| `lab/world/dial.go` | D-class | Home constructor/inspection may initialize or migrate: Open | contracts, trust |
| `lab/world/disk.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | trust |
| `lab/world/frameproxy.go` | D-class | Home constructor/inspection may initialize or migrate: Open | trust, contracts |
| `lab/world/pairing.go` | P-class | Product implementation linked in-process: trust | trust |
| `lab/world/relay.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | trust |
| `lab/world/truth.go` | wire-contract only | Contract types/codecs only; replace with lab-owned wire before deleting Go | contracts |
| `lab/world/world.go` | D-class | Product home/database mutation or fixture setup; audit before Rust qualification | contracts, trust |

## G-E continuation after PR #540

The baseline table above remains a historical inventory, not a current pass count. The Mac gateway paths in `mac.go`, `mac_actions.go`, `mac_clipboard.go`, `mac_commands.go`, `mac_extras.go`, and `mac_extras_real.go` now use a lab-owned readback oracle. Agent setup uses the selected gateway CLI, and command schemas come from the selected binary's OpenAPI endpoint instead of `MacDescriptors`/`OperationSchema`. Existing assertion predicates are retained; schema-fetch success is additionally asserted. Go SIM proof covers MAC-ACTIONS, MAC-COMMANDS, MAC-EXTRAS-SIM and the SIM branch of IWORK-REAL. No real-level run or Rust pass is implied.

`AUTH-SCOPES-METADATA` additionally uses the frozen OAuth scope corpus and selected-binary token creation to test registration, followed by the unchanged metadata assertions; its Go SIM proof passes. `agent_activity_capture.go` creates concurrent reader/writer agents through the selected gateway CLI, with WEB-AGENT-ACTIVITY proved on Go; the enclosing owner-session setup remains coupled and blocked for Rust. The frame-aware fault proxy now honors blackholes in both directions, preserving the existing CORE-DISCONNECT-DETECT assertion.

`blob_journey.go` uses the frozen contract progress type and read-only post-migration inspection. CORE-BLOB-GC preserves offline legacy-index fault injection followed by selected-binary restart and migration, proved against Go with its existing assertions.

Remaining P/D conversions are not complete. Concrete missing fixture contracts and their affected families are in `rust/interfaces/lab-world.md`. Product imports in the remaining files continue to block their Rust qualification; changing an inventory label does not convert a journey. Shadow tool semantics, reproducible synthetic proofs and unresolved live-run requirements are in `lab/shadow/README.md`.


## G-E binary owner-session continuation

`agent_access.go`, `agent_activity.go`, `agent_levels.go` and `agent_level_boundaries.go` now use the selected gateway/replica CLIs and normal software WebAuthn enrollment instead of opening Go Authority instances. Enrollment uses the frozen cookie name and owner tokens remain inside disposable homes. The same Go assertions pass for consent, OAuth, CIMD, capture observers, activity, levels and replica delegation. Activity-age and command-clock fixtures remain explicit lab-owned data faults; no product constructors implement them.

Only WEB-AGENT-ACTIVITY and WEB-AGENT-LEVELS receive the new Rust-gateway SIM entry allowance. Their callback roots and common wire-client files are explicitly inventoried, avoiding unrelated branches of the shared web dispatcher while retaining import/semantic-helper checks. Their real-level and Rust-replica combinations remain outside this allowance. The existing rate-window function is preserved byte-for-byte in `web_rate_limits_fixture.go`; its Go Handler/RateClock coupling remains unconverted. WEB-AUTH-RATELIMIT and WEB-AUTH-SOFTWARE have no registered YAML in this tree; private attempts to run those dormant bodies failed their existing assertions and are not qualification evidence.


## G-E binary OAuth continuation

`agent_credentials.go`, `agent_oauth_credentials.go`, `agent_oauth_grants.go` and `agent_oauth_http.go` now drive the selected CLI and public registration/consent/token/revocation boundaries. OAuth wire models and token hashing are lab-owned. Existing resource/expiry corruption remains an explicit frozen-format fixture fault; authentication is verified through the selected HTTP endpoint and principal lookup is read-only. The before/after Go assertion lists and verdicts match exactly: 46 credential assertions and 28 OAuth assertions. Both journeys and consent pass against Go before their Rust SIM entry allowance is extended. No real-level or Rust-replica allowance is changed.

Remaining custom CIMD resolver/transport, public-origin/security, iPhone runtime and in-process state/performance fixtures are still unqualified. No constructor or HTTP handler from the Go product is invoked by the converted OAuth paths.

## G-E source/capture and blob-wire continuation

`source_grants.go` now uses selected CLI agents, normal HTTP owner/OAuth setup, and SimApplePeer `/capture` IPC instead of Go State/Authority constructors or a lab-client source-registration side effect. `capture_observers.go` creates approval agents through the selected gateway CLI. The original named Go assertion verdicts match the previous binary: source grants 30, capture observers 23, blob GC 43, blob retry 34, PDF annotation 14, PDF text 13 and reminders demo 111. The demo's independent observer ordering can vary; its named verdict map is unchanged.

A lab-owned JSON digest-reference decoder replaces `contracts.BlobReferences` in the PDF simulator and gateway blob/media journeys. Codec vectors preserve derivative order, duplicate references, lowercase digests and invalid-document handling. This removes that semantic product helper; it does not qualify other product imports in the same files. Only source grants and capture observers gain additional Rust-gateway/Go-replica SIM entry allowances after their Go proofs. No native or real-level allowance changes.

## G-E delegation-wire continuation

`delegation.go` now authenticates its replica read through the selected HTTP boundary, reads the frozen principal/certificate fixture, and constructs the deliberately scope-expanded command with lab-owned command-hash and Ed25519 delegation codecs. It no longer opens a Go Authority or calls product signing/hash semantics. Golden JSON/hash and delegation payload/signature vectors independently verify wire compatibility. CORE-FORGED-DELEGATION passes against Go with the original unauthorized/no-effects predicate. As before, that predicate does not identify the exact verifier rejection stage; no stronger security claim is made. Other delegated-agent and negative-authority journeys remain separate conversions.
