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

Ported journey: `command_journey` for CORE-CMD-IDEMPOTENT (scoped REST/MCP queries and commands on gateway and both replicas, idempotent retry, `id_reused`, issuer isolation, command-page pagination, truth/gateway/replica convergence, effect counts, replica cursor and record consistency). The Go SSE watcher assertions (`sse_contiguous_watermarks_*`, `no_false_deletion_*`, `scope_sse_watermarks_*`) and the unverified `replica_application_exactly_once` are not ported; `families-parity.py` ignores exactly those names for journey scenarios. Rust convergence compares per-collection title multisets with SimApple truth rather than the Go alias-resolved record multiset.

Not ported: the other eight CORE-CMD-* cases (need helper/gateway stop and restart, clock offset, command backup/restore, response-drop and feed-delay faults), `blobs` journeys (CORE-BLOB-*, need native staging, blob fault controls and replica blob flags), media transfer, history-import scheduling. These fail explicitly with `not ported`. Source-family simulators belong to S-P8-PIM and S-P8-MEDIA.

Gate: `python3 rust/lab/families-parity.py --families $T/lab-families --go-lab bridge-lab --gateway BRIDGE --simapple $T/lab-simapple --output /new/dir`, once with a `-tags lab` Go gateway and once with the Rust gateway built with `--features lab-queries`.

### Live seeded-fault gates (gateway binary, not traces)

Stable API additions, no change to `WorldConfig`, `World` or `Registry`: the peer control route `/protocol` returns `{opens:[{fence, offered:{source:checkpoint}, commits_seen}], commits:[{source, checkpoint}], acked:{source:checkpoint}}`, and the runner step `protocol_gate` takes `check: reopen-offers-committed` or `check: acked-data-visible`. A world's child processes inherit the runner's environment, so a seeded fault is selected by `AB_LAB_SEEDED_FAULT=<name>`; no `WorldConfig` field was added.

The Rust gateway has cargo feature `seeded-faults` on `ab-runtime` (implies `lab-harness`). Only a binary built with it reacts to `AB_LAB_SEEDED_FAULT`; the product and `lab-queries` builds contain no fault code. Implemented faults: `skip-checkpoint-commit` (the gateway offers no checkpoint at reopen, as if the commit never became durable) and `ack-uncommitted-data` (the gateway sends Committed without storing the capture).

Scenarios `LIVE-CHECKPOINT-REOPEN` and `LIVE-ACK-DURABLE` are the behavioral observers. `python3 rust/lab/live-fault-gate.py --runner R --healthy-gateway H --mutant-gateway M --simapple S --output SHORT_DIR` requires each scenario to pass on the healthy gateway and the named assertion, with no infrastructure assertion failing, to fail on the mutant. The output path must be under 48 characters because worlds create Unix sockets inside it.

Verified on Linux (`ssh build`): three consecutive runs, healthy passes and both mutants detected. Not done: `accept-stale-fence` and `leak-scope` still only have the protocol-trace observers (they need a peer control route that sends a frame on a superseded connection, and a scoped-token query observer), and the Go harness has not been run against the seeded-fault gateway, so "caught by both harnesses" is not yet shown for any fault. `make rust-ci` passes fmt, clippy and tests; `cargo deny` fails only on RUSTSEC-2024-0449 (serde_yaml in lab-runner), which S-DENY is replacing.
