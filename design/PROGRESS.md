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
- DONE blobs round 2 (91af0cc): incremental protection, durable retry budget; 55 core sim pass.
- DONE Calendar (853ec2f): read + write; FSEvents WAL hint beats EventKit's ~1.17 s notification lag; external create p95 real 704 ms, Ubuntu 854 ms.
- Contacts merged (18bb050): 11 sim + 5 real pass, real p95 254 ms; replica CON-EXTERNAL pass (p95 482 ms); replica CON-CRUD fixed (94a216b: forwarding used a hard-coded reminders:write scope). contacts.read/write + reminders.write DONE.
- Messages commands merged (36be10a, c250164): self text/files pass sim/real/replica; incoming echo real+replica; history import 161,121 msgs in 381 s, 0 mismatches → messages.read + messages.send DONE. Actions/groups LIMITED at real level (screen lock; no 2nd iMessage account). File cap removed (cb41f84: staged blob paths; 5 MiB self-send verified, 696 ms). Test-message cleanup awaits unlock.
- DONE Notes (56b6570): 5 sim, 4 real + 2 Ubuntu replica pass while locked; independent JXA Truth and unique tagged folders; external p95 real 590 ms / replica 555 ms. Notes restart cleared a stalled Automation queue; one earlier transient replica delete failure remains recorded in lab/evidence/notes-automation-diagnosis.json.
- DONE Usage/Screen Time: read-only Biome source, independent Truth, lock/day math + scrubbed CRLF fixtures; five sim scenarios and USE-IMPORT real pass (1,246 app/day records, zero mismatches); absent DevicePeer names are explicit, historical sleep/settings parity remain estimates.
- LIMITED packaging.app (3658b73): signed Apple Bridge.app, SMAppService agent, dedicated keychain (works with login Keychain locked); PKG-INSTALL + PKG-BACKGROUND real pass; notarization needs Johan's Developer ID; GUI onboarding clicks and reboot untested.
- DONE agent suite (31891ec): Codex gpt-6-luna on Ubuntu, MCP only, 7 tasks across 6 sources, each 2/2, graded by independent Truth.

## Status (2026-10-17 final)
- features.yaml: 20 done, 4 limited, 0 todo (175/183 scenario-levels passing; the 8 not passing are the listed limits). Lint ok. No worktrees left except none; branch `usage-wip` (unmerged leftover) kept.
- Open follow-ups (not blocking): command responses > 256 KiB over the gateway-dials feed; delete tagged self-test messages once the screen is unlocked; Notes Automation can stall (relaunch Notes) — consider auto-recovery.

## Limits
- messages.actions/groups real: Accessibility needs an unlocked screen (Johan's password); iMessage group management needs a second iMessage account.
- core.trust CORE-CRED-LIFECYCLE/real: login Keychain locked (needs Johan's password).
- packaging.app notarization: needs Johan's Developer ID.
