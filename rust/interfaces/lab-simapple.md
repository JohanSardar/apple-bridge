# Rust SimApple foundation

Binary: `lab-simapple simapple --socket PATH --secret-file PATH --control PATH`.

Select it in the existing Go harness with `AB_LAB_SIMAPPLE=/absolute/path/to/lab-simapple`. Initial launch, StopHelper and RestartHelper all select the same executable. Existing scenario files are unchanged. Suite evidence retains gateway/replica implementation fields and adds `conditions.simapple_impl` and the selected executable path. This switch is for a Rust peer; leave it unset for the Go baseline.

The independent peer authenticates IPC, consumes owner fences and byte credit, emits source snapshots, validates matching Committed receipts, and chains checkpoints offered by Open on reconnect. It holds arbitrary synthetic observations by native key for all sources and serves seed, inspect, external-change, dataset, capture, capture-atomic, source-state, effects, command-origins, stats, flush and close over an owner-only HTTP Unix socket. Records and effects persist in rust-sim-state.json through an fsynced rename; dispatch guards are fsynced before effects. Unfinished guards report indeterminate before startup captures.

The command foundation covers reminders and basic messages send/reply, crash-after-effect, crash-before-outcome, ambiguous-outcome, identical-manual-send, UI interference before/after prepare, and pause. Message seed, permission loss/restore and generation-change controls are present. It is not a complete port of the roughly 6,900-line Go simulator: source-specific Calendar/Contacts/Notes and other command/control adapters, query/blob/media simulation, incremental scan progress, live priority and the full fault catalog still require implementation and parity gates. Do not replace the default Go peer yet. Unsupported command/control routes fail rather than supply fabricated observations. Unknown fault names fail explicitly; only the faults above are implemented.

Reproduce the bounded foundation parity gate on Linux:

```sh
cargo build --manifest-path rust/Cargo.toml -p lab-simapple
go build -o /tmp/bridge-lab ./cmd/bridge-lab
python3 rust/lab/parity.py --go-lab /tmp/bridge-lab --rust-peer "$CARGO_TARGET_DIR/debug/lab-simapple" --output /tmp/phase8-parity
```

The gate runs six unchanged binary-driven scenarios against the Go gateway and Go replicas, using each peer three times. It requires success and identical per-assertion verdicts. Scenarios cover idempotency, command crash and ambiguity, eight gateway kills after capture commit before acknowledgment, identical manual send, UI interference, and permission loss with another source remaining available. This is peer-foundation evidence, not Phase 8 completion or full Rust-harness mutant coverage.

Foundation verification: all 36 executions passed with identical assertion verdicts across both peers and all three repetitions. Raw JSONL, binary hashes and assertion digests are in `lab/evidence/phase8-foundation/`. `make rust-ci` and seven independent codec tests passed on build.

## Messages read extension

The Rust peer additionally supports incoming messages, receipts, malformed-body digests, native deletion, watcher rearm/poll controls and message status. Seeded messages retain the Go fixture titles and Unicode content. Permission restoration captures the current native snapshot. Message database/schema replacements change only the Messages generation and reset its checkpoint predecessor. Capture chunks declare reference aliases first and order absences last with completion evidence, matching the Go peer.

On Linux, one run of the unchanged MSG-RECEIPTS, MSG-DB-REPLACED and MSG-PERMISSION-LOSS YAML scenarios passed for both peers with identical assertion verdicts (six executions). The same three scenarios also pass with `--impl gateway=rust` and the Rust peer. Evidence is in `lab/evidence/phase8-messages-read/`. Reproduce with:

```sh
python3 rust/lab/parity.py --go-lab /tmp/bridge-lab --rust-peer "$CARGO_TARGET_DIR/debug/lab-simapple" --output /tmp/messages-read-parity --scenario messages/MSG-RECEIPTS.yaml --scenario messages/MSG-DB-REPLACED.yaml --scenario messages/MSG-PERMISSION-LOSS.yaml
```

The parity runner defaults to one repetition; `--repetitions 3` reproduces the foundation repetition count. It collects both peers' assertion failures before rejecting the gate and validates the exact scenario identities.

This is a bounded peer extension, not a complete Messages family or Rust scenario-runner port. MSG-INCOMING failed its one-second latency assertion in the Go baseline on the loaded build host. History-import still needs bulk pause/release and capture statistics. Decoder and watcher scenario gates invoke native macOS checks and cannot complete on Linux. Messages command operations beyond the foundation, other source adapters, query/blob/media simulation and the broader fault catalog remain. The four Phase 8 seeded gateway faults have not been gated against independent Go and Rust harnesses; those gates remain required before declaring a family complete or deleting Go.

The host-quiet wait is already opt-in: `bridge-lab run` defaults to sim without requiring quiet, and `--require-quiet` sets the context request used by `lab/scenarios/telemetry.go`. Without that explicit flag the 60-second wait is bypassed, including for latency scenarios.

## Messages commands extension

The Rust peer now ports the Go Messages command behaviour: send, reply, react, edit, unsend, mark_read, mark_unread, delete_message and delete_chat, with tagged-text and own-recipient rules, the 15-minute edit and 2-minute unsend windows (recomputed from `sent_at` by message facts on every message write, including `/external-change`), expected-state conflicts, one outstanding ticket, and the origin and principal checks. The `/messages` control adds `nonself-chat`, `capability-unproven`, `capability-proven` and `capability-proven-empty-reason`; the Ready frame advertises all nine operations with per-operation coverage. Faults added: `delivery-failed`, `ghost-send`, `receipt-before-outcome`, `outcome-before-capture`, `outcome-without-capture`, `part-no-effect`, `part-delivery-failed`, `part-progress`, `part-progress-failure`, `part-progress-stale`, `part-receipt-failed`, `part-receipt-failed-first` and `part-unbound-failure`. Resolve upgrades `locally_observed` to `delivered` and applies part progress.

Peer parity against the Go peer (`rust/lab/parity.py`, Go harness, Go gateway) on Linux: MSG-CAPABILITY-MATRIX, MSG-COMMAND-OBSERVED-IDS, MSG-DELETE, MSG-EDIT, MSG-FAILED-OBSERVED-IDS, MSG-MARK-READ and MSG-OWNER-TOKEN-SELFCHAT identical; MSG-COMMAND-PARTS identical except `delayed_capture_no_prolonged_unknown` (one-second latency), which failed in the Go baseline under host load 30 and passed with the Rust peer.

## Independent runner and world API (Phase 8)

The stable extension boundary is `lab-world` plus `lab-runner`, with no `ab-*` dependencies. All interaction with products is through selected executables, public HTTP, or SimApple's control socket. The runner never executes `bridge-lab`.

`lab-world` exports `WorldConfig { gateway: PathBuf, simapple: PathBuf, directory: PathBuf, scope: String }`, `World::start(WorldConfig) -> Result<World>`, `World::control(&self, route: &str, input: &Value) -> Result<Value>`, `World::query(&self, operation: &str, input: &Value) -> Result<Value>`, `World::gateway_cli(&self, args: &[&str]) -> Result<String>`, `World::restart_gateway(&mut self) -> Result<()>`, and `World::close(&mut self)`. Public fields `directory`, `gateway_home`, `runtime` expose synthetic run paths and runtime JSON. Drop closes child processes; files remain for diagnosis. Each world must use a fresh private directory. HTTP calls and readiness waits have deadlines. Only sim level is supported initially; unsupported actions fail explicitly.

`lab-runner` exports `Scenario` (YAML fields `id`, `feature`, `levels`, `given`, `when`, `then`, `cleanup`), `Assertion { name, passed, detail }`, `Evidence { scenario, passed, assertions }`, and `Registry`. Adapter signature: `fn(&mut World, &serde_json::Value, &mut Evidence) -> lab_world::Result<()>`. Register with `Registry::register(name, handler)`; execute with `Registry::run(&Scenario, &mut World) -> Evidence`. Each YAML step is a single-key map. Handlers must preserve original assertions, reject unsupported variants, and add named assertions using `Evidence::check(name, passed, detail)`. Cleanup always runs, including after action failure. Scenario cleanup `world` closes all processes.

CLI: `lab-runner run SCENARIO.yaml ... --gateway /path/to/bridge --simapple /path/to/lab-simapple --output /private/run/root`. One JSON evidence object per scenario goes to stdout; any failure exits nonzero. Gateway selection is the supplied executable (Go or Rust), never a linked product crate. Adapter modules should register additional action names; extending the peer's control routes does not change this boundary.

The four required mutant names are `skip-checkpoint-commit`, `accept-stale-fence`, `ack-uncommitted-data`, and `leak-scope`. A mutant gate must first prove its healthy control passes, then require the same behavioral observer to reject the seeded violation; infrastructure failure cannot count as detection. Until both independent harnesses have that evidence, Phase 8 remains incomplete.

### Runner implementation and bounded verification

`lab-runner run` implements the published extension API. Built-in steps are `world` (zero replicas only), `control` (`route`, `input`), `query` (`operation`, `input`, optional exact `equals`), `restart_gateway`, `capture_truth`, and `fault_trace`. It requires `curl` for bounded HTTP/Unix control transport. Unknown actions and replica worlds fail. `capture_truth` currently supports a bounded page of simple observations; family adapters must supply their original reference, pagination, tombstone, latency, replica and scope assertions before claiming parity for existing YAML families.

`RUST-CAPTURE-RESTART` and `RUST-CAPTURE-UPDATE` drive a selected gateway and the Rust peer from YAML, checking public capture visibility, update visibility, and persistence across a gateway kill/restart. These new smoke scenarios do not replace existing family scenarios.

Four `LAB-TRACE-*` YAML scenarios run the same healthy and seeded protocol event traces through independently implemented Go and Rust observers. They catch a missing checkpoint on reopen, acceptance with an old owner fence, an ack without a preceding commit, and a returned source outside the allowed scope. Evidence explicitly labels these as protocol trace sensitivity checks. They are not live gateway mutation gates. Connecting real gateway fault injection and authenticated protocol observations to these invariants remains required for Phase 8 acceptance.

Reproduce with `python3 rust/lab/runner-parity.py --runner /path/to/lab-runner --go-lab /path/to/bridge-lab --gateway /path/to/bridge --simapple /path/to/lab-simapple --output /new/private/output`. This runs six Rust-runner scenarios and the four Go trace scenarios, checks verdict parity for the four named faults, and records binary hashes. Run once with each selected gateway binary. It does not invoke the Go harness from the Rust runner.

## Family adapters (Phase 8, query and blob)

`rust/lab/lab-adapters` has no `ab-*` dependency and produces `lab-families`, which wraps `lab_runner::Registry::standard()` with family adapters and the same CLI as `lab-runner`, leaving the runner files untouched. Adapters register through `lab_adapters::registry()`; a later merge may move them into `lab-runner/src/main.rs`.

Ported: the `queries` action for CORE-QUERY-ROUNDTRIP (ten echoes, latency budget, REST/MCP parity, audit sizes only, invalid deadline rejected), CORE-QUERY-BLOB (digest, range, scope), CORE-QUERY-SCOPE (scoped token, agent tokens, approval-gated write) and CORE-QUERY-DEADLINE (bounded deadline, REST and MCP cancellation by killing the client call). The `world` action accepts `replicas` only for these two scenarios, whose sim steps use the gateway alone. Any other scenario with replicas fails explicitly. Assertion names and order match the Go harness; the Go-only `credential_backend_override_restored` assertion has no Rust equivalent and is ignored by the parity script.

The Rust SimApple peer now answers `Query` (`test.echo`, `test.sleep`, `test.blob`, `helper.version`) and `BlobRead` with `BlobChunk` frames, and queues them during a capture commit wait. `test.sleep` runs on a worker thread that observes `QueryCancel` and the query deadline; `/effects` reports 1 for its query id while it runs and 0 afterwards, and the session loop keeps serving frames.

Replica worlds live in `lab-adapters` (`replicas.rs`): the `world` action pairs, joins, tokens and starts `replicas` replica processes from the selected gateway executable, waits for `connected`, and `lab-families` stops them after each scenario. `seed` and `converge` are registered; `lab-world` and `lab-runner` are untouched.

Ported journey: `command_journey` for CORE-CMD-IDEMPOTENT (scoped REST/MCP queries and commands on gateway and both replicas, idempotent retry, `id_reused`, issuer isolation, command-page pagination, truth/gateway/replica convergence, effect counts, replica cursor and record consistency). The SSE observers (`sse_contiguous_watermarks_*`, `scope_sse_watermarks_*`), `no_false_deletion_*` and `replica_records_versions_and_cursor_consistent` run through the shared assertion layer in `lab-adapters` (`assertions.rs`, `watch.rs`). `converged` is alias-resolved: truth records are mapped through the gateway alias table, `$ref` values are resolved to record ids, and the per-record multiset is compared with the gateway, then replicas are compared with the gateway records visible to their scope at one cursor, using the Go `recordMultiset` keys. Every journey uses `Observed`, `converged`, `sse_observers`, `stream_assertions` and `record_integrity`. `replica_application_exactly_once` is reported `unverified` exactly like Go when no kill-during-apply check ran; `assertions::kill_during_apply` ports the Go check (arm `lab-apply-armed`, wait for `write_started_before_commit`, crash and restart the replica, then replica doctor `exactly_once`) and is passed to `record_integrity` by journeys that request it. No scenario in the current gate calls it. Assertions now carry Go's optional `status`, and `families-parity.py` compares `(passed, status)` for every assertion with no per-name ignores; only the harness-only `credential_backend_override_restored` and the Rust-only `world_started` are excluded. `no_false_deletion_*` checks each observed delete against SimApple truth at the end of the run, stricter than the Go in-stream oracle.

Ported with command backup/restore, response-drop and feed-delay controls: CORE-CMD-ERA (`bridge backup`/`restore`/`--accept-ledger-loss`, ledger removal and the `ledger_missing` start check, epoch rotation, SSE observers resuming in the new epoch) and CORE-CMD-LOST-RESPONSE (`drop-command-response` in the gateway home, `command-apply-delay` in a replica home honoured by the Rust replica feed under `lab-harness`). `World` gained `stop_gateway` and `start_gateway`, and a gateway restart reuses its peer and API addresses so replicas redial. Replica worlds now hold `reminders:write` so commands can be submitted through a replica, as in the Go world. CORE-CMD-CONFLICT (two replicas race `reminders.update` at one version: one winner, one `conflict` with current, no effect for the loser) and CORE-CMD-READ-YOUR-WRITE (local read at `commit_seq`, the gateway SSE observer's capture sequence equals `commit_seq`, no-op command commits at a fresh observation) run through `command_journey` with no new controls. Not ported: the remaining CORE-CMD-* cases (helper restart, clock offset, expiry), `blobs` journeys (CORE-BLOB-*, need native staging, blob fault controls and replica blob flags), media transfer, history-import scheduling. These fail explicitly with `not ported`. Source-family simulators belong to S-P8-PIM and S-P8-MEDIA.

Also ported in `restart.rs`: CORE-CMD-CRASH-AMBIGUOUS, CORE-CMD-EXPIRY, CORE-CMD-CLOCK-JUMP, CORE-CMD-OFFLINE and CORE-CMD-CONFLICT. `lab-world` gained `stop_helper` and `clock_offset` (writes `clock-offset` in the gateway home, read by the Rust gateway's lab clock); helper restarts use `restart_peer` and gateway restarts reuse the previous listen addresses. The Rust peer now derives ticket deadlines from the prepare frame's expiry with millisecond precision, matching the Go peer, instead of truncating to whole seconds.

Gate: `python3 rust/lab/families-parity.py --families $T/lab-families --go-lab bridge-lab --gateway BRIDGE --simapple $T/lab-simapple --output /new/dir`, once with a `-tags lab` Go gateway and once with the Rust gateway built with `--features lab-queries`.

### Live seeded-fault gates (gateway binary, not traces)

Stable API additions, no change to `WorldConfig`, `World` or `Registry`: the peer control route `/protocol` returns `{opens:[{fence, offered:{source:checkpoint}, commits_seen}], commits:[{source, checkpoint}], acked:{source:checkpoint}, stale:{stale_fence, current_fence, reply, key, collection, data}}`; the peer control route `/stale-frame` (input `key`, `collection`, `data`) opens two extra authenticated connections so the first is superseded by the second, sends a Capture on the superseded one, and records the gateway reply (`committed`, `closed`, `silent`, `rejected`); the runner steps are `protocol_gate` (`check: reopen-offers-committed`, `acked-data-visible` or `stale-fence-rejected`) and `scope_gate` (`scope`, `inside`, `outside` collections: a token created with the narrow scope must read the inside collection and nothing from the outside one, while the full-scope token sees the outside records). A world's child processes inherit the runner's environment, so a seeded fault is selected by `AB_LAB_SEEDED_FAULT=<name>`; no `WorldConfig` field was added.

Cargo feature `seeded-faults` exists on `ab-runtime` (implies `lab-harness`) and forwards to `ab-operations` and `ab-applepeer`. Only a binary built with it reacts to `AB_LAB_SEEDED_FAULT`; the product and `lab-queries` builds contain no fault code. Implemented faults: `skip-checkpoint-commit` (no checkpoint offered at reopen), `ack-uncommitted-data` (Committed sent without storing the capture), `accept-stale-fence` (a superseded connection is not aborted and its captures skip the active-fence check) and `leak-scope` (read visibility is widened to admin for every principal).

Scenarios `LIVE-CHECKPOINT-REOPEN`, `LIVE-ACK-DURABLE`, `LIVE-STALE-FENCE` and `LIVE-SCOPE-QUERY` are the Rust behavioral observers. `python3 rust/lab/live-fault-gate.py --runner R --healthy-gateway H --mutant-gateway M --simapple S --go-lab G --output SHORT_DIR` (run from a checkout, `--go-repo` defaults to it, the Go harness writes evidence under it) requires each Rust scenario to pass on the healthy gateway and the named assertion, with no infrastructure assertion failing, to fail on the mutant. It then runs the Go harness (`--impl gateway=rust`) on the Go scenarios in `GO_GATES`, each of which must pass on the healthy gateway and fail its named assertion on the mutant. A fault with no Go scenario fails the gate. The output path must be under 48 characters because worlds create Unix sockets inside it.

Go coverage measured on `ssh build` (healthy passes, mutant fails on the named behavioral assertion; a failure at `world_start`, `action` or `peer_ack_observed` never counts as detection): `skip-checkpoint-commit` by `CORE-HELPER-RESTART` (`helper_restart_checkpoint_drained`) and `CORE-KILL-GATEWAY-CAPTURE` (`capture_replay_converged_iteration_0`); `accept-stale-fence` by `CORE-STALE-FENCE` (`stale_fence_rejected`); `ack-uncommitted-data` by `CORE-ACK-DURABLE` (`acked_data_visible`); `leak-scope` by `CORE-SCOPE-LEAK` (`all_registered_scope_paths`). Three consecutive gate runs at load 11 to 17 had the healthy gateway pass and all four mutants rejected by both harnesses.

The Go SimApple has the same `/stale-frame` control route as the Rust peer (input `key`, `collection`, `data`; it opens two authenticated connections, sends a Capture on the superseded one and returns `stale_fence`, `current_fence`, `reply`). `CORE-STALE-FENCE` calls it and requires the reply not to be `committed` and the record to be absent from `records.query`. `CORE-ACK-DURABLE` starts its world through `world.WithoutAcknowledgedCapture`, which skips the seed and the wait for the first acknowledged capture, sends one external change, waits until SimApple `/stats` shows the revision committed, then requires the record to be visible through `records.query`. `given: world: {replicas: 0}` is accepted by scenario validation for these replica-free worlds.

## Media derivation (Phase 8)

The Rust peer derives attachment variants with no `ab-*` dependency: nearest-neighbour resize, thumbhash, MP4 geometry parsing, source-sized posters, clip and audio fixtures (`rust/lab/lab-simapple/assets/`, extracted from the Go constants). `messages.send` with `files` (path, name, digest) creates an attachment message per file with `derivatives` and `media`, stores the original and variant blobs, serves them through the existing `BlobRead` handler, and advertises them as `blob_handles` on Messages captures. Decoders are `zune-png`, `zune-jpeg` and `gif`; encoders are `jpeg-encoder` and a small PNG writer on `miniz_oxide`, chosen so the native offline dependency graph is unchanged (`image` enables `flate2/zlib-rs`). `IJG` is added to the cargo-deny allow list for `jpeg-encoder`.

Unit tests compare thumbhash, geometry, poster dimensions, MIME sniffing and variant dimensions with vectors computed from the Go peer. Variant JPEG/PNG bytes differ from Go, so digests are not byte-identical; scenario assertions do not compare them. Variant blobs are session-local, so they do not survive a helper restart. WEB-MEDIA-VARIANTS, WEB-MEDIA-CAP and MSG-MEDIA-CLEANUP still need the media-transfer adapters (not on main).

## Media transfer adapters (Phase 8)

`lab-adapters` registers `media_variants` (WEB-MEDIA-VARIANTS, WEB-MEDIA-CAP) and `messages_commands` (MSG-MEDIA-CLEANUP only). `media_variants` starts one replica itself (`replicas::start_with` takes the scope and `--blob-cap`), stages a generated 2400x1200 PNG, the landscape video fixture and the audio fixture through `/v1/blobs`, sends them with `messages.send`, then checks per-endpoint variant capture, authorized blob bytes, digests, image dimensions, handle removal, the cap state, survival across a SimApple restart (`World::restart_peer`), and the source-geometry sends. Assertion names match the Go harness.

The WEB-MEDIA-VARIANTS cleanup proof and MSG-MEDIA-CLEANUP shell out to the native `MessagesSafetyChecks` binary (located through `AB_LAB_NATIVE_OUTPUT` or `native/.build/native-output-1`). Without it, both fail with `native_cleanup_checks_build`, exactly as the Go harness does on a host without the Swift build, so they only pass on Darwin.

WEB-MEDIA-CAP passes with identical verdicts on the Rust gateway since `fix(replication): defer originals while a media variant is still pending`.

## Calendar completion (Phase 8)

The Rust peer now implements the remaining Calendar behaviour of the Go simulator: `future` span updates and deletes with series splitting, the `move` control, the ±730-day window with `evict` observations (also applied to `/inspect`), the injected stamp match for `server_update`, the 15-minute verification clock, and the `calendar-rule-mismatch` fault. Captures list calendars before events so a chunked capture never references an event calendar the gateway has not seen; the demo dataset load previously stalled on that and CON-RECIPIENT-LOOKUP failed. `effect_at` is taken before the capture is enqueued, as in the Go peer.

On Linux with `--level sim --local-replicas`, all 25 Calendar scenarios give identical per-assertion verdicts for the Go and Rust peers in one run. CAL-STORE-READ, CAL-HINT-COVERAGE, CAL-INVALIDATED-EVENT and CAL-FUTURE-COUNT-MATH fail identically on both because they call macOS tooling (`xcrun`). Contacts 14/14, Notes 9/9 and Reminders 11/11 are identical.

## History import and Darwin media (Phase 8)

The Rust peer implements `bulk-pause` (key = native key of the held row) and `bulk-release` on `/messages`. While paused, a chunked Messages capture stops before the 64-row part that contains the held key, `/messages-status` reports `bulk_blocked`, and live requests (`receipts`, `incoming`) are served between polls as delta captures that wait for their Committed. A bulk `seed` returns without waiting while paused. Every Messages part is revalidated against current state when it is finally sent, so a stale bulk snapshot cannot regress a live value. `/messages-status` also reports `capture_count`, `capture_ack_ms`, `capture_bytes` and `credit_waits`.

`lab-adapters` registers `messages`, `messages_read` (history-import only), `messages_truth`, `contacts` (CON-IMPORT), `notes` (NOTE-IMPORT) and `usage` (USE-IMPORT). Replica worlds for these four use one replica with the read scope of every source, and the gateway observer uses the same scope so replica integrity compares like with like. The feed observers reuse `watch.rs`; `Watcher` gains `upserts_of` and `data_for_title`.

Gate: `python3 rust/lab/families-parity.py --families $T/lab-families --go-lab bridge-lab --gateway BRIDGE --simapple $T/lab-simapple --output /new/dir --import` runs MSG-HISTORY-IMPORT, CON-IMPORT, NOTE-IMPORT and USE-IMPORT in one run against both harnesses. Verdicts are identical with the Rust gateway (`lab-queries`) and with the Go `-tags lab` gateway on Linux.

`--media` runs WEB-MEDIA-VARIANTS and MSG-MEDIA-CLEANUP the same way and needs the native `MessagesSafetyChecks` binary (`AB_LAB_NATIVE_OUTPUT`), so it runs on Darwin. Verdicts are identical on the MacBook Pro: MSG-MEDIA-CLEANUP passes its 19 fixture cases in both harnesses, WEB-MEDIA-VARIANTS passes all 98 shared assertions. Two adapter bugs were fixed to get there: the sent video and audio fixtures lacked the trailing `free` box that Go's `MediaFixture` appends (so the panorama source geometry fell back to the default clip), and the panorama poster was fetched once instead of awaited, with no pixel checks. The adapter now decodes the poster (`zune-png`, `zune-jpeg`) and ports the centre-marker and edge-contrast checks.

## Web read-model adapters (Phase 8)

`lab-adapters` registers `readmodels` (`readmodels.rs`, `search.rs`) for WEB-RM-MEDIA-COLLECTION and WEB-RM-SEARCH. Scenarios with no `when` step in YAML (WEB-RM-SEARCH) get a `readmodels` step injected by `lab-families`. The adapters seed through the SimApple `/readmodels` put route, resolve gateway aliases, call REST and MCP, mint scoped tokens, and for the search migration stop the gateway, rewrite `records_fts` and `fts_version` in `state.db` with rusqlite, and restart it. Assertion names match the Go harness (WEB-RM-SEARCH emits no `world_start` on success, as in Go).

Gate: `python3 rust/lab/families-parity.py ... --web-rm` runs both scenarios against both harnesses; verdicts are identical (23 and 47 assertions).

## Mac scenarios

`lab-adapters` registers the `mac` step (`mac.rs` for sim, `mac_real.rs` for real-level adapters). The Rust SimApple peer gained a `mac` source (`mac.rs`): `/mac` upserts one `mac.*` record and emits a full-scan capture for the source, `/mac-notifications` sets the notification permission fixture (`mac.notify` is rejected `forbidden` unless authorized), `mac.*` commands answer with `{observed, fingerprint, readback}` evidence, and `mac.clipboard_read`, `mac.network_check`, `mac.bluetooth_devices`, `mac.time_machine_status` and `mac.recognize_song` answer the Go fixtures. The clipboard text lives in memory only so it never reaches a durable file.

Gate: `python3 rust/lab/families-parity.py ... --mac` runs MAC-ACTIONS, MAC-COMMANDS, MAC-CAPTURE, MAC-IDLE, MAC-EXTRAS-SIM and IWORK-REAL (sim level) in one run against both harnesses.

Real-level only (adapters compiled, never run at real level): BT-REAL, MAC-APPEARANCE-REAL, MAC-APPS-REAL, MAC-AUDIO-REAL, MAC-CLIPBOARD-REAL, MAC-KEEPAWAKE-REAL, MAC-NOTIFY-REAL, MAC-POWER-REAL, MAC-REPLICA, SHAZAM-REAL, TM-STATUS. The real halves of MAC-IDLE (`mac_real::idle`) and IWORK-REAL (`mac_real::iwork`) are also compiled only. The native-helper adapters read the helper app from `APPLE_BRIDGE_HELPER_APP`.

## Web, media and native scenarios (Phase 8)

`--web` runs WEB-CON-LIST and WEB-MEDIA-CAP; `--native-media` runs NATIVE-MEDIA-EXPORT-SAFETY, NATIVE-MEDIA-POSTERS, NATIVE-MEDIA-YIELD and WEB-REM-PARITY. Both flags use the same one-run gate as `--import` and `--media`.

`contacts_list.rs` is dispatched from `readmodels` for WEB-CON-LIST: one replica with the contacts, messages and system read scopes, six synthetic fixtures through `/external-change`, then `contacts.list` over REST and MCP on the gateway and the replica (membership sections, name/identity/photo, linked conversations, group counts, device preferences, invalid input, scoped tokens and deletion counts).

`native.rs` shells out to the Swift check binaries through `packaging/dev/build-native.sh` and `native/.build/native-output-1`, so it runs on Darwin only. The three NATIVE-MEDIA scenarios also run the selected-gateway fixture check (`selected_gateway_*`): two versions and a tombstone for one `messages.attachment` posted to the SimApple `/capture` route and read back through `records.query`. The runner treats a scenario with no given/when/then steps as one implicit step named after the scenario id, which is how WEB-REM-PARITY (and any later step-less scenario) selects its adapter. `Scenario` accepts the optional `web` block that WEB-REM-WRITE carries.

Not ported: WEB-AGENT-ACTIVITY, WEB-AGENT-CREDENTIALS, WEB-AGENT-LEVELS, WEB-AGENT-OAUTH and WEB-AGENT-OAUTH-CIMD need an owner web session (a software WebAuthn authenticator: CBOR, P-256, enrollment and login ceremonies, session cookie and CSRF) that the Rust lab does not have yet. WEB-CAL-WRITE needs the CAL-CRUD calendar protocol journey. WEB-REM-WRITE is real-level only.

## Real-level runner (Phase 8)

`lab-runner` and `lab-families` accept `--level sim|real` (default `sim`) and `--helper-app APP`. `--simapple` is only required at sim level.

The real level mirrors Go's `world.NewReal` protections and is compile-only in CI: nothing in the Phase 8 work runs it.

- `--level real` refuses to start unless `--helper-app` names a bundle with `Contents/MacOS/AppleBridgeHelper` that passes `codesign --verify --strict` and carries a `TeamIdentifier` (ad-hoc signatures are rejected). The refusal happens before any output directory or gateway exists.
- `World::start_real(RealConfig)` creates a fresh `0700` directory, refuses to run when it is inside, equal to or above the live home (`~/Library/Application Support/AppleBridge`, `$XDG_DATA_HOME/apple-bridge`, `~/.local/share/apple-bridge`), starts the gateway with `--home <directory>/gateway --helper-app APP --helper-lab-commands`, and waits for `helper/helper.pid`. There is no SimApple peer and no `sim.sock`, so `World::control` is unavailable.
- `Registry::register_real(id, handler)` registers a handler `fn(&mut World, &mut Evidence) -> Result<()>` for a scenario ID. `Registry::run_real(scenario, RealConfig)` fails the scenario with `real_adapter` (no world started) when the scenario does not list `real` (MAC-REPLICA and FILE-REPLICA list `replica` and count as `real`, as in Go) or has no registered handler.
- Real-only scenarios are reachable by adding one `register_real` line in `lab-adapters/src/lib.rs`. Handlers mint scoped tokens with `Client::new(world, scope)`; the world token scope is `location:read,maps:read,maps:write,files:read,files:write,safari:read,system:read`.

Unit tests cover the refusals (no Helper, unsigned Helper, live home) and an unrelated directory.

## Lab relay/2 listener (Phase 8, web relay prerequisites)

Both the Rust gateway (`lab-harness`, which `lab-queries` and `seeded-faults` imply) and the Go gateway built with `-tags lab` can expose their relay/1 and relay/2 feed over TLS 1.3 to the lab. Product builds contain neither the Rust module nor the Go file's body; the Go product build links a no-op `serveRelayLab`.

The listener is configured only by `<home>/lab-relay.json`, read at gateway start. If the file is absent nothing is listened on:

```json
{"listen": "127.0.0.1:0", "cert": "/abs/path/leaf-chain.pem", "key": "/abs/path/leaf-key.pem"}
```

`cert` is the server chain (leaf first) signed by a lab-owned test CA; `key` is the PKCS#8 private key. The lab owns the CA (`lab-relay` generates it with rcgen) and its clients trust only that CA, never webpki roots. After binding, the gateway writes the resolved `host:port` to `<home>/run/lab-relay-addr`. A gateway restart rereads the file and rebinds, so read the address again.

Only `GET /v1/feed` is served; any other path returns 401. The handler is the production relay handler (`feed.RelayHandler()` in Go, `Server::relay_http` in Rust): no client certificate is requested, requests carrying an `Origin` header are rejected with 401, relay/1 hellos (no `version`) are accepted as legacy, relay/2 hellos are bound to the TLS 1.3 exporter `EXPORTER-Channel-Binding` with an empty context. The feed started after authentication is the same one the gateway peer and ingress paths use.

## Maps, location and Files handlers (Phase 8)

The Rust peer answers the Maps and Location queries of the Go simulator (`maps.rs`): `location.current`, `location.geocode`, `location.reverse_geocode`, `maps.search`, `maps.nearby`, `maps.directions`, `maps.eta` and `maps.snapshot`, with the Go argument rules (strict keys, coordinate and radius ranges, `transit` directions `unsupported`, snapshot size 128..1024, at most 50 markers). Snapshots are blank RGBA PNGs stored in the session blob map. `maps.open` prepares and dispatches with the `com.apple.Maps` frontmost evidence bound to the request hash. The peer advertises `maps.open` and the eight `files.*` commands in `Ready`.

`files.rs` serves `files.search` and `files.read` from `files.item` records and `files.create|write|move|rename|trash|set_tags|download|evict` for root `/Bridge Test`, emitting a complete `files.item` capture before the `Outcome`; content is stored base64 in `data.blobs[digest]`, the same store `/dataset` fills, so `BlobRead` serves it.

Real-level runner additions: `Registry::register_standalone(id, handler)` registers a real-level scenario that needs no world (native checks, packaging); `World::helper_app` carries the signed Helper path of a real world; `World::sibling(suffix)` starts a second sim world next to the first (FILE-COMMANDS runs its REST and MCP passes in separate worlds, as the Go harness does).

Gate: `python3 rust/lab/families-parity.py ... --maps` runs MAP-SIM, LOC-SIM, FILE-COMMANDS, FILE-CRUD-REAL and FILE-OFFLOAD-REAL (sim level) in one run against both harnesses; verdicts are identical with the Rust gateway (`lab-queries`).

## Safari, FILE-REPLICA and packaging adapters (Phase 8)

`safari.rs` ports SAF-CAPTURE and SAF-IDLE (sim): the adapter writes the synthetic Safari databases with the same Python fixture as the Go harness, builds and runs the native `SafariChecks`, checks the six observations against the registered collection shapes, posts them to the peer `/safari` route, and compares gateway and replica records. Both are in the `--native` parity set and need macOS.

Compile-only real adapters: SAF-IDLE (`idle_real`), SAF-BOOKMARKS-REAL and SAF-HISTORY-REAL (standalone; native checks against an independent Python truth), FILE-REPLICA (`filerep.rs`, local replica instead of the Ubuntu replica) and PKG-INSTALL/PKG-BACKGROUND (`pkginstall.rs`, standalone). None of these has been run.

QUERY-AGENT and AGENT-SUITE are not ported: they drive a Codex agent over MCP (`lab/agent/codex.py`) and need an LLM, its credentials and the agent task suite.

## Native scenario runner (Phase 8)

`lab-adapters/src/native_runner.rs` ports Go's `runNativeScenario` (`lab/scenarios/native.go`), which `lab/scenarios/runner.go` dispatches by scenario ID, not by YAML step. Ported at sim level: REM-CRUD (both `core/REM-CRUD.yaml`, which uses `command_journey`, and `reminders/REM-CRUD.yaml`, which uses `reminders_crud`; they share one ID and run the same native flow, REST then MCP with `mcp_`-prefixed assertion names), REM-CREATE-EXTERNAL, REM-LIST-MANAGE, REM-LIST-CREATE, REM-RECURRING and REM-PERMISSION-LOSS. The step names `reminders_external`, `reminders_crud`, `reminders_list`, `reminders_recurring` and `reminders_permission_loss` all call `native_runner::run`, and `journey::command_journey` and `journey::seed` hand native IDs over or ignore them. A native world is seeded with `/seed {count: 0}` and one replica regardless of the YAML `replicas` or `seed` values, exactly like Go's `world.New(ctx, level, 1, 0)`. The no-op `fixture` cleanup action is registered because the sim level has nothing to clean.

Reusable pieces for other families: `native_runner::uptime_ns()` and `native_runner::instant_ns(Instant)` are one monotonic nanosecond clock shared by effect stamps and the SSE `Watcher` (which records an `Instant` per change, so `first_seen_title` converts through `instant_ns`); `native_runner::records_at(client)` is Go's `RecordsAt` (records plus the first-page cursor, erroring with `snapshot changed during pagination` if the cursor moves); `native_runner::replicas_equal_gateway(gateway, replicas)` is the `RecordsAt` replica comparison (equal cursor and equal `system.command`-free record lists); `native_runner::without_commands`. The peer controls `/complete-recurring` and `/source-state` already exist in `lab-simapple`. The clock is process-local, which is exact at sim level; peer-supplied `effect_uptime_ns` values from AppleTruth belong to the real level, which is never run.

REM-LIST-LOST-ACK is real-only: `native_runner.rs` compiles its flow (arming `helper/lab-pause-outcome`, the ambiguous response, the paused-outcome marker and list visibility), and reports `native_list_effect_before_lost_outcome` as `unverified` because the native list inventory needs AppleTruth. It is never run at real level.

Other Go scenarios that reach `runNativeScenario`, so their families should build on `native_runner.rs` rather than re-port it: HELPER-LIVE-BACKFILL, HELPER-SESSION-SOAK, HELPER-RESTART-REAL and HELPER-TCC-READY (all real-only; they need `Truth`, `RestartHelper`, helper-home pause files and the signed helper app, so compile-only), and CORE-LIVE-LATENCY at the real level (its sim and replica levels use the `changes` action instead). Their shared prologue (`reminders_ready`, `ready`) and epilogue (`replica_convergence`, per-endpoint `no_scope_leak`) are `Native::protocol` and `Native::epilogue`.

Gate: `python3 rust/lab/families-parity.py ... --reminders` now also runs the seven native files above (`core/REM-CRUD` and `reminders/REM-CRUD` both included). `verdicts()` keys a repeated scenario ID with a trailing `#`, so duplicate IDs are compared pairwise in order. Verdicts are identical for the Go harness and the Rust harness against the Rust gateway on Linux.

## Owner passkey session (`lab-adapters/src/webauthn.rs`)

Scenarios that end in an owner approval need an owner session, and the gateway has no CLI to mint one. The Rust lab ports the Go virtual WebAuthn authenticator into a lab-owned module with no `ab-*` dependency (`p256`, `sha2`, `serde_json`, `curl`):

- `Authenticator` holds a P-256 key, a random 32-byte credential ID, a counter and a user-verified flag. `credential(create, challenge, origin, rp)` builds the same `none`-attestation registration and ES256 assertion the Go authenticator builds (hand-written CBOR, internal transport, user handle `apple-bridge-owner`).
- `Owner::enroll(&Gateway)` runs `bridge web enroll --host http://localhost:PORT`, drives `/auth/enroll/options` and `/auth/enroll/finish` over HTTP with an `Origin` header, and keeps the `ab_session_local` cookie. `Owner::login()` runs the discoverable-login ceremony with the enrolled key. `Owner::decide(id, "approve"|"reject")` posts the `/approvals/{id}` form and parses the command record out of the status page.
- The ceremony is the real one: the gateway verifies the attestation, stores the passkey, and verifies the assertion signature and counter. APP-CMD-SIM and APP-NOTIFY-SIM use it for `owner_device`, `owner_authenticated` and `iphone_owner_approval_without_helper`.

## iPhone command and capture lab hooks

`lab-harness` builds of the gateway add these actions on the lab socket for the iPhone capture scenario, which Go drives in-process:

- `phone-delegated` clones a command into an admitted-prepared-dispatched row for a delegated principal.
- `phone-delegated-mode` (`allow <request_hash>` or `deny`) controls the delegated re-authorization check in the phone gateway adapter. `phone-delegated-authorized` reports whether that check ran for the allowed row.
- `phone-blob-grant` reports whether a principal holds a query-blob grant for a digest.

Production builds contain none of this.

## Vision, PDF, audio and capture (Phase 8)

The Rust peer answers `vision.ocr`, `pdf.text`, `pdf.search`, `audio.transcribe` and `speech.synthesize` with the Go fixtures (three OCR PNG digests, the fixture PDF digest and any `pdf.artifact` blob reference, the 364-byte WAV), and runs `pdf.annotate`, `speech.say`, `capture.play_sound`, `capture.screenshot`, `capture.camera_photo`, `capture.record_audio`, `mac.screen_record` and `print.document` (the capture operations intercept before the mac family, as in Go). `pdf`, `speech`, `print` and `capture` are advertised in Ready but are not scanned sources. `lab-adapters/src/vision.rs` ports AUD-SIM, PDF-ANNOTATE, PDF-TEXT, VIS-OCR and CAP-SIM, and compiles AUD-TRANSCRIBE, SPEECH-SAY and VIS-IDLE (real-only; the runner rejects them at sim level). Gate: `families-parity.py --vision` on Linux (four scenarios) and `--vision --vision-native` on macOS, where CAP-SIM builds `CaptureChecks`.

## Real-level iPhone qualification (`lab-adapters/src/iphone_phone_real.rs`)

APP-SHELL-REAL, APP-PAIR-REAL, APP-CMD-REAL, APP-NOTIFY-REAL, APP-APPROVE-REAL, APP-LIVE-REAL and APP-INTENTS-REAL are ported from Go's `runIPhoneReal`. They are compile-only and unit-tested on pure parts (pairing-code base32, receipt parsing, requirement tables); no phase 8 work runs them.

- `lab-families --level real` routes these IDs (and the sensor IDs in `iphone_real.rs`) to `iphone_real::run_real` before any world exists. The signed-Helper requirement applies only when the run includes a scenario that is not handled by the iPhone real path, so a phone-only run needs no Helper.
- The run needs `AB_REAL_DEVICE`, `APPLE_BRIDGE_SECRET_STORE=file` and `APPLE_BRIDGE_IPHONE_ROOT`. It takes the `/tmp/ab-iphone.lock` phone lock, drives the iPhone project's `lab/iphone-real/device.py` and `devicectl` (request copy, launch, receipt polling with the offline-time allowance, 210 s per phase, 800 s total), and always runs the restore handshake.
- Paired scenarios start an isolated `Gateway` fixture. A `Lan` forwarder listens on `0.0.0.0` with its own TLS certificate, hands connections to the fixture's ingress proxy, and rewrites the pairing code's `address` and `fingerprint` for the phone (`AB_REAL_HOST` or `en0`).
- Push scenarios point the gateway at a loopback relay stand-in that forwards `POST /iphone/push` to the APNs sandbox with an ES256 token (`AB_REAL_APNS_KEY`, `AB_REAL_APNS_KEY_ID`, `AB_REAL_APNS_TEAM_ID`) and counts accepted and failed pushes for `apns_accepted`.
- APP-APPROVE-REAL's connector phase registers an OAuth client, creates a pending connector approval, and after the phone decision exchanges the code and checks the token against the API. The pending-approval response field names (`id`, `code`) and the token check operation are unverified against a live run.
- The paired device ID is read from the gateway's `credentials.json` because the gateway has no CLI to list iPhones.
