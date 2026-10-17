# Progress

Resume point for any fresh session. Updated after every unit of work.

## Environment facts (verified 2026-10-17)
- Air: macOS 26.7.1 (25G241), SIP on, 8 GB, Swift 6.3.3 CLT, Go 1.27.2 (Homebrew), ~15 GB free disk (tight).
- SSH session on the Air has Full Disk Access: chat.db (WAL), NoteStore.sqlite, Biome restricted streams readable.
- chat.db: services iMessage, SMS, RCS present; schema includes date_edited, date_retracted, message_summary_info, associated_message_emoji.
- No code-signing identities on the Air (`security find-identity` = 0). notarytool/stapler exist in CLT. Developer ID needed for M5 notarization.
- Air → Ubuntu: `ssh ubuntu` works (key air_to_ubuntu). Ubuntu → Air: `ssh air`. Tailscale: Air 100.78.200.121, Ubuntu 100.80.96.36.
- Ubuntu: no Go (replica binaries are cross-compiled on the Air), codex-cli 0.163.0 present.
- Biome restricted streams on this build: App.InFocus, ScreenTime.AppUsage, UserFocus.*, Screen.Sharing; no Display/ScreenLock stream found by name (M4 must find clipping evidence elsewhere).

## Done
- Step 1: design/SPEC.md, GLOSSARY.md, PRINCIPLES.md, docs/adr/0001–0014, features.yaml, tools/featuregraph (checker + HTML), tools/lint (zero comments + import boundaries), Makefile.

## Verified
- `go run ./tools/lint` ok; `go run ./tools/featuregraph` ok (24 features, 0/177 scenario-levels passing).

## In flight (2026-10-17)
- DONE `ab-m0-tracer-review-r1`: tracer scenarios reproduced (sim LIVE p95 12.6–15.3 ms over 3 runs; Ubuntu p95 100 ms). Findings: runner Truth keyed by title (hides duplicates), unconditional transactional assertion, malformed scenario options default silently, SSE deadline + cursor identity, status typing → `ab-m0-tracer-fix-r1`; partition = process kill and SSH-tunnelled replica path → already in M0-C brief; typed IDs (raw strings) → deferred refactor after M0.
- DONE `ab-m0-tracer-fix-r1` (3581bc3..852eb09): all six review findings fixed; self-checks LAB-TRUTH-MULTIPLICITY, LAB-MALFORMED-SCENARIO, LAB-SSE (sim). Latency outliers under heavy Air contention: sim max 1.8 s; one replica run p95 3.0 s (rerun p95 98 ms). Must be explained before claiming the <1 s target (CORE-BENCH + quiet-machine runs). Reviewed by `ab-m0-tracer-fix-review-r1`: 0 findings on both axes (adversarial probes passed; sim rerun replica LIVE p95 36 ms). Open evidence gaps: kill-during-apply exactly-once (M0-C), latency causation — next latency work must record host telemetry (load, CPU, concurrent heavy jobs) in evidence and include quiet-machine runs.
- SSH ControlMaster multiplexing to the Air broke (mux Broken pipe) and looked like host overload; use `ssh -o ControlPath=none air` if ssh stalls. Air load average spikes (~100) come largely from Johan's Chrome/headless Chrome, not only our workers.
- `ab-m0-commands-r1` (worktree m0-commands): command lifecycle, ApplePeer command frames, SimApplePeer commands + faults, RemoteGateway + delegated identity, MCP, CORE-CMD-*, parity, forged delegation, localhost defense, REM-CRUD/CONFLICT/EXTERNAL-RACE/READONLY-LIST.
- `ab-m0-robust-r1` (worktree m0-robust): fault proxy, direct Tailscale replica path, snapshot/retention/gap/kill/slow-consumer/disconnect/upgrade/helper-restart/restore/disk-full/scope/pair/cred-lifecycle/bench scenarios.
- DONE `ab-m0-blobs-r1` (5f418b8..0b03412): blobs store, eager IPC reads, resumable prioritized feed transfer, record-authorized REST, caps, protection, GC. CORE-BLOB-INTERRUPT/CORRUPT/CAP/NO-DIGEST-AUTH/GC + new OFFLOADED/SLOW-READ pass at sim; INTERRUPT at replica (5.9 s, n=1). SPEC updated (blob_handles, chunk header, disk_full). Reviewed by `ab-m0-blobs-review-r1`: no digest-only access bypass, lab corruption hook absent from product build; findings P1 gateway abandons blob downloads after IPC failure (reproduced), P2 GC and lane-priority scenarios could pass vacuously; 5.9 s = 4 MiB fixture + 150 ms/chunk throttle + 5 s heartbeat-driven retry. Fix round `ab-m0-blobs-fix-r1` running (also pushes blob availability instead of heartbeat). Known: repo-wide `make sim` trips on a real-helper YAML action `cleanup.fixture` (M1-A's).
- `ab-m1-session-r1` (worktree m1-session): native build fix, Swift AppleSession IPC, AgendaGate + RemindersSource, Helper supervision, AppleTruth app, RealAirEnvironment, REM-* real/replica, HELPER-*.
- `ab-m1-messages-read-r1` (worktree m1-messages-read): Decoders (typedstream, summary info, reactions), MessagesSource capture, MSG-* read scenarios.

## Next
- Review each M0 unit when it lands (fresh reviewer per unit); route confirmed findings to fix rounds.
- M1 Helper-session unit (Swift IPC client for ipc/1, AgendaGate, Reminders/Calendar sources, AppleTruth, build fix via swiftly) once M0-B has defined command frames.
- Real-level work once TCC grants appear in the `status` report.

## Done (M0 tracer)
- Tracer `ab-m0-tracer-r1` (5b083db..d4e5ce4): CORE-BOOT, CORE-LIVE-LATENCY, CORE-RESUME pass at sim and replica (Ubuntu via SSH tunnel). Replica CORE-LIVE-LATENCY p50/p95/max = 19.8/109.6/169.0 ms (n=200); sim 8.6/10.9/15.1 ms. Dependency approved: golang.org/x/sys (peer UID).

## M1 facts (2026-10-17)
- TCC (verified by parent in TCC.db and by `status`): FDA, Accessibility, Automation Messages + System Events, Reminders granted to AppleBridgeHelper (DR: identifier dev.applebridge.helper + certificate leaf 889be0a2…). Calendar, Contacts pending: Johan will run the fixed onboard (28a90dd) later; detect via `status`, don't ask again.
- iMessage AppleScript send to self works with raw `any;-;` chat id (delivered); normalized `iMessage;-;` → -1728. Outgoing row text NULL (attributedBody only) + incoming echo row in self chat.
- `ax-dump` works (308 nodes) but redacts identifiers too; the Messages-actions unit should dump non-personal AX attributes (roles, identifiers, actions) unredacted.

- Calendar/Contacts root cause (fixed a0d917a): hardened runtime needs `com.apple.security.personal-information.calendars/.addressbook` entitlements; without them TCC refuses silently (no prompt, no TCC.db row). Verified prompt now appears (tccd AUTHREQ_PROMPTING). Johan will run onboard later; detect via `status`.
- Possible interference: a foreign process `imsg` (pid 54735, likely Johan's old Toy/Hermes setup) was seen spamming TCCAccessRequest to tccd every ~4 ms; it was not running at 00:05. Do not kill such processes without asking.
- heavy.sh is a FIFO queue (2 slots) since a0d917a.
- Replica level: SSH tunnel acceptable until Johan's Tailscale ACL fix; direct Tailscale path behind a World option.

## GUI control (2026-10-17)
- VNC GUI control of the Air via `~/tools/airvnc.py` (see BUILD-GOAL update and WORKER-RULES). Leader handles all TCC dialogs and Settings toggles; no more manual requests to Johan unless the lock screen persists > 1 h.

## Blockers
- TCC grants for AppleBridgeHelper (asked 2026-10-17; Johan will do it when at the Air; don't ask again). Detect by running the `status` mode (see WORKER-RULES) between work units; start real-level work once granted.
- Expected later: Developer ID for notarization (M5).
