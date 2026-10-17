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
- `ab-m0-tracer-review-r1`: two-axis review of the tracer (282d82b..d4e5ce4), reruns sim + replica.
- `ab-m0-commands-r1` (worktree m0-commands): command lifecycle, ApplePeer command frames, SimApplePeer commands + faults, RemoteGateway + delegated identity, MCP, CORE-CMD-*, parity, forged delegation, localhost defense, REM-CRUD/CONFLICT/EXTERNAL-RACE/READONLY-LIST.
- `ab-m0-robust-r1` (worktree m0-robust): fault proxy, direct Tailscale replica path, snapshot/retention/gap/kill/slow-consumer/disconnect/upgrade/helper-restart/restore/disk-full/scope/pair/cred-lifecycle/bench scenarios.
- `ab-m0-blobs-r1` (worktree m0-blobs): internal/blobs + feed transfer + IPC blob reads + CORE-BLOB-*.

## Next
- Review each M0 unit when it lands (fresh reviewer per unit); route confirmed findings to fix rounds.
- M1 Helper-session unit (Swift IPC client for ipc/1, AgendaGate, Reminders/Calendar sources, AppleTruth, build fix via swiftly) once M0-B has defined command frames.
- Real-level work once TCC grants appear in the `status` report.

## Done (M0 tracer)
- Tracer `ab-m0-tracer-r1` (5b083db..d4e5ce4): CORE-BOOT, CORE-LIVE-LATENCY, CORE-RESUME pass at sim and replica (Ubuntu via SSH tunnel). Replica CORE-LIVE-LATENCY p50/p95/max = 19.8/109.6/169.0 ms (n=200); sim 8.6/10.9/15.1 ms. Dependency approved: golang.org/x/sys (peer UID).

## Blockers
- TCC grants for AppleBridgeHelper (asked 2026-10-17; Johan will do it when at the Air; don't ask again). Detect by running the `status` mode (see WORKER-RULES) between work units; start real-level work once granted.
- Expected later: Developer ID for notarization (M5).
