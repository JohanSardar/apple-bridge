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
