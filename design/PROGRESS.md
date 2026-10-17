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
- Messages read: sim done; real runs merged (a78a5b8): HISTORY-IMPORT real FAILED (no convergence in 1,002 s, suspect blob protection), INCOMING pending (no natural arrivals) → both handed to the Messages commands worker.
- DONE blobs round 2 (91af0cc): incremental protection, durable retry budget; 55 core sim pass.
- Calendar merged (44dfd7d): 14 sim, real/replica CRUD, real RECUR-THIS pass → calendar.write DONE. calendar.read open: external-create latency 1.3 s (target <1 s) + 9 empty Bridge Test calendars left → fixing.
- Contacts merged (18bb050): 11 sim + 5 real pass, real p95 254 ms; replica CON-EXTERNAL pass (p95 482 ms); replica CON-CRUD fixed (94a216b: forwarding used a hard-coded reminders:write scope). contacts.read/write + reminders.write DONE.
- IN PROGRESS Messages commands (send, AX actions, groups) — `.worktrees/m1-messages-commands`; real AX = pending-unlock while screen locked.
- Notes merged (d2f5c83): all sims + NOTE-IMPORT real pass. Real/replica writes fail: Notes AppleScript -1712 (Helper + AppleTruth; AppleTruth Notes/MobileSMS AppleEvents rows added 2026-10-17) → `ab-notes-real-r1` diagnosing; else limited.
- IN PROGRESS Usage/Screen Time (`ab-m4-usage-r1`).
- LIMITED packaging.app (3658b73): signed Apple Bridge.app, SMAppService agent, dedicated keychain (works with login Keychain locked); PKG-INSTALL + PKG-BACKGROUND real pass; notarization needs Johan's Developer ID; GUI onboarding clicks and reboot untested.
- IN PROGRESS agent suite (`ab-agent-suite-r1`): Codex gpt-6-luna on Ubuntu via MCP, Reminders + Contacts first.

## Limits
- core.trust CORE-CRED-LIFECYCLE/real: login Keychain locked (needs Johan's password).
- packaging.app notarization: needs Johan's Developer ID.
