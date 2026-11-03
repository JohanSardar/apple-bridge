# Rust SimApple foundation

Binary: `lab-simapple simapple --socket PATH --secret-file PATH --control PATH`.

Select it in the existing Go harness with `AB_LAB_SIMAPPLE=/absolute/path/to/lab-simapple`. Initial launch, StopHelper and RestartHelper all select the same executable. Existing scenario files are unchanged. Suite evidence retains gateway/replica implementation fields and adds `conditions.simapple_impl` and the selected executable path. This switch is for a Rust peer; leave it unset for the Go baseline.

The independent peer authenticates IPC, consumes owner fences and byte credit, emits source snapshots, validates matching Committed receipts, and chains checkpoints offered by Open on reconnect. It holds arbitrary synthetic observations by native key for all sources and serves seed, inspect, external-change, dataset, capture, capture-atomic, source-state, effects, command-origins, stats, flush and close over an owner-only HTTP Unix socket. Records and effects persist in rust-sim-state.json through an fsynced rename; dispatch guards are fsynced before effects. Unfinished guards report indeterminate before startup captures.

The command foundation covers reminders and basic messages send/reply, crash-after-effect, crash-before-outcome, ambiguous-outcome, identical-manual-send, UI interference before/after prepare, and pause. Message seed, permission loss/restore and generation-change controls are present. It is not a complete port of the roughly 6,900-line Go simulator: source-specific Calendar/Contacts/Notes and other command/control adapters, query/blob/media simulation, incremental scan progress, live priority and the full fault catalog still require implementation and parity gates. Do not replace the default Go peer yet. Unsupported command/control routes fail rather than supply fabricated observations. The generic fault endpoint currently retains its Go string interface; only the faults above are implemented.

Reproduce the bounded foundation parity gate on Linux:

```sh
cargo build --manifest-path rust/Cargo.toml -p lab-simapple
go build -o /tmp/bridge-lab ./cmd/bridge-lab
python3 rust/lab/parity.py --go-lab /tmp/bridge-lab --rust-peer "$CARGO_TARGET_DIR/debug/lab-simapple" --output /tmp/phase8-parity
```

The gate runs six unchanged binary-driven scenarios against the Go gateway and Go replicas, using each peer three times. It requires success and identical per-assertion verdicts. Scenarios cover idempotency, command crash and ambiguity, eight gateway kills after capture commit before acknowledgment, identical manual send, UI interference, and permission loss with another source remaining available. This is peer-foundation evidence, not Phase 8 completion or full Rust-harness mutant coverage.
