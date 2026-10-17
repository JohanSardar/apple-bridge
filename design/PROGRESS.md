# Progress

One line per unit. Rules: BUILD-GOAL.md (2026-10-17 update: no reviews, merge on green, trimmed tests), WORKER-RULES.md.

## Environment
- Air: macOS 26.7.1, SIP on, 8 GB, Go 1.27, Swift 6.3.3 CLT (SwiftPM broken → `packaging/dev/build-native.sh`), heavy jobs via `packaging/dev/heavy.sh` (2 slots).
- Access: `ssh -o ControlPath=none air` (reverse tunnel 127.0.0.1:2222); Ubuntu→Air direct impossible, Air→Ubuntu direct works. VNC: `~/tools/airvnc.py`. Screen locked tonight (Accessibility UI unavailable while locked).
- TCC: Helper (dev.applebridge.helper) and AppleTruth (dev.applebridge.truth) hold Reminders/Calendar/Contacts (user TCC rows, backups in `~/Library/Application Support/AppleBridgeDev/`); Helper also FDA, Accessibility, Automation. Never change the dev signing cert; per-run app copies only.
- Johan unreachable: identity-bound items → `limited` with reason.

## Units
- DONE spec + tooling: SPEC, glossary, ADRs, features.yaml + checker, zero-comment lint.
- DONE M0 core: State, replication, commands (opaque IDs, ledger.db), REST+MCP, blobs, trust, restore, faults — 9 core features done; core.trust limited (real Keychain locked).
- DONE gateway-initiated replica connections (43d11f1): direct Air→Ubuntu LIVE p95 109 ms. Deferred: command responses > 256 KiB over the feed (P2 from a cancelled review).
- DONE Helper session + Reminders read (27e49ad): real native-save → Ubuntu p95 317 ms; 10-min soak clean.
- DONE Messages read path sim (61e03e7); real runs in progress (`ab-messages-real-runs`).
- IN PROGRESS blobs round 2 (incremental protection, durable retry budget) — `.worktrees/blobs-fix2`.
- IN PROGRESS Calendar (branch m1-calendar): 7 fixes, real/replica runs, merge.
- IN PROGRESS Contacts (branch m2-contacts): real journeys passed; merging.
- IN PROGRESS Messages commands (send, AX actions, groups) — `.worktrees/m1-messages-commands`; real AX = pending-unlock while screen locked.
- NEXT: Notes (M3), Usage/Screen Time (M4), AgentRunner + AGENT-SUITE, packaging (M5; notarization limited without Developer ID; Keychain-in-background decision).

## Limits
- core.trust CORE-CRED-LIFECYCLE/real: login Keychain locked (needs Johan's password).
- Notarization: needs Johan's Developer ID.
