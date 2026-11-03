# Phase 3 CRUD investigation, 2026-11-02

Harness prerequisite PR #543 was reviewed, rebased onto current main, tested with `go test ./lab/scenarios ./lab/world` and source lint on Ubuntu, and merged as 39e45547. No real-level scenario was run in this investigation.

The three existing CRUD journeys now run at sim level with SimApplePeer, Go gateway and Rust replica. REM-CRUD gains a sim level routed to the same REST/MCP native protocol journey. Assertions are unchanged. The mixed sim qualification exception is an explicit scenario allowlist; Rust gateways remain blocked for these journeys.

CAL-CRUD, REM-CRUD and NOTE-CRUD all passed with the original Rust implementation, then again with a freshly built current-main Rust binary. This does not reproduce the reported real-level failures and does not establish a product fix for them.

## Calendar

Real evidence `/tmp/ab-phase3-real-20261102/recheck/rust/CAL-CRUD/evidence.json` fails no_false_deletion_0 and _1. Observer 0 watches the Go gateway and observer 1 the Rust replica. Both call the same live AppleTruth oracle. Replica convergence and full records/versions/tombstones integrity pass in that evidence. The assertion failure remains serious, but the evidence cannot locate its cause in Rust apply/install: the source feed also fails it. No oracle or assertion was weakened.

`replica_application_exactly_once` is explicitly unverified in CAL-CRUD because that journey calls `recordIntegrity(..., nil)` and never performs a kill-during-apply probe. It is also unverified in the Go run. The separate unchanged CORE-KILL-DURING-APPLY scenario exposed a missing Rust lab hook: its first write-before-commit boundary timed out before this change. ab-state now implements that boundary behind its existing `lab` feature, enabled by ab-runtime/lab-harness. It consumes the armed marker, writes the reached marker with mode 0600, and parks inside the transaction before applied_batches/cursor commit. Production builds have no hook. The unchanged scenario then passed all eight process crashes, recovery convergence, and exactly-once SQLite checks. This proves crash recovery for that scenario; it does not turn CAL-CRUD's absent probe into a pass.

## Reminders

Real evidence `/tmp/ab-phase3-real-20261102/rust/REM-CRUD/evidence.json` contains only a boolean convergence failure, with no cursor or record differences. All command and visible-state assertions pass. The existing sim journey passes REST and MCP unchanged. Added safe final cursor/read/equality metrics to REM-CRUD so the next orchestrator-owned real run can distinguish lag/read failure from record mismatch. No speculative apply/batching change was made without a reproduction.

## Notes

Real evidence `/tmp/ab-phase3-real-20261102/rust/NOTE-CRUD/evidence.json` returns at native_fixture, before import or replica checks. Its preserved private report `AppleBridgeDev/lab-failures/99y2rjts0t/truth_01M6FM5B3HJGR295RCSJ6XSR6Z-report.json` identifies NotesJavaScript error -10000, AppleEvent handler failed, on action=create. The request's test tag belongs to this run. The fixture failed in AppleTruth before Rust exercised the note. Owned cleanup counts matched, but global baseline rose 134 to 136. No real fixture creation or cleanup was attempted here. Added the already-redacted Truth failure code to native_fixture detail; raw native error text remains private.

## Environment and evidence

Builds and sims ran on ssh ubuntu; Go used ~/go-sdk/bin. sccache was absent. The first fresh Rust build failed with a /tmp disk quota; its own temporary target was removed and subsequent builds used ~/ab-cargo-target and TMPDIR=~/ab-phase3-tmp. No build artifacts are committed.

Go CAL-CREATE-EXTERNAL, REM-CREATE-EXTERNAL and CON-CRUD subsecond latency failures on the heavily loaded Air remain environment-sensitive and were not pursued.

Phase 3 real qualification remains incomplete: Calendar false-deletion and Reminders convergence require an attributable reproduction; Notes requires successful native fixture creation and verified cleanup on a quiet Air. This change must not be reported as fixing all three native failures.

Additional unchanged mixed sim Calendar probes all passed: CAL-MOVED-OUTSIDE-WINDOW, CAL-RECUR-FUTURE, CAL-RECUR-THIS, CAL-SPLIT-ATOMIC-ALLDAY, CAL-SPLIT-ATOMIC-TIMED, CAL-WINDOW-SLIDE. These exercise eviction versus deletion, occurrence identity, future splits, and window slides. Their explicit mixed sim qualification was enabled alongside CRUD; no assertion changed.

Ubuntu evidence is retained outside git at `~/ab-phase3-sim-before.json`, `~/ab-phase3-sim-current.json`, `~/ab-phase3-faults.json`, `~/ab-phase3-crash-after.json`, `~/ab-phase3-calendar-faults.json`, and `~/ab-phase3-rust-ci.log`. The initial faults file intentionally retains the failing crash-boundary result; the after file is the passing eight-crash regression.

Repository verification passed on Ubuntu: `make rust-ci` (source registration/lint, go vet, workspace Rust tests, formatting, strict Clippy, dependency advisories/bans/licenses/sources, Go golden checks), `cargo clippy --manifest-path rust/Cargo.toml -p ab-runtime --features lab-queries --all-targets -- -D warnings`, and `go test ./lab/scenarios ./lab/world`.
