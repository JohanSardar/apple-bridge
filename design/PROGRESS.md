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
- features.yaml: 20 done, 4 limited, 0 todo (175/183 scenario-levels passing; the 8 not passing are the listed limits). Lint ok. No worktrees left; unmerged leftover branch `usage-wip` kept (superseded by the merged Usage source).
- Open follow-ups (not blocking): command responses > 256 KiB over the gateway-dials feed; delete tagged self-test messages once the screen is unlocked; Notes Automation can stall (relaunch Notes) — consider auto-recovery.

## Limits
- messages.actions/groups real: Accessibility needs an unlocked screen (Johan's password); iMessage group management needs a second iMessage account.
- core.trust CORE-CRED-LIFECYCLE/real: login Keychain locked (needs Johan's password).
- packaging.app notarization: needs Johan's Developer ID.

## Follow-up (2026-10-17, unlocked-session brief)
- packaging.app DONE: R-PKG now makes notarization optional; fresh PKG-INSTALL/real and PKG-BACKGROUND/real pass. Initial background startup health failure retained; cause unproven. Current feature totals: 21 done, 3 limited (175/183 scenario-levels passing).
- Messages actions/groups remain LIMITED: live captures still show a password field and root owns `/dev/console` despite `IOConsoleLocked=No`. Seven scenarios skip native actions; no UI fix or distinct-participant claim is proven. Remy configured; no messages sent. Login Keychain still reports `User interaction is not allowed`; lifecycle not run. An active unlocked GUI session and unlocked login Keychain are still needed, without supplying passwords to workers.
- Cleanup retained 30 tagged message candidates (GUI unavailable) and one provably empty calendar (created by an earlier run, forbidden to delete under this-run ownership rule). Details and exact commands: `lab/evidence/messages-unlocked-recheck.json`; screenshots: shared checkout `lab/evidence/private/unlocked-recheck/` (gitignored). This entry supersedes earlier environment/limit notes; no new scenarios.

## Authorized follow-up (2026-10-17)
- DONE earlier-build empty calendar cleanup: explicit Johan authorization, independent unbounded enumeration found one Bridge Test calendar with zero events; signed AppleTruth removed it and a fresh unbounded audit found zero calendars. Evidence: `lab/evidence/messages-real-session-cleanup.json`. `make lint` and `make features` pass.


## Messages finish attempt (2026-10-18)
- Login-Keychain lifecycle now PASS without UI: process-local Security interaction is disabled before credential calls; create/read/rotate/revoke takes 302 ms (one sample, p50/p95/max equal). No keychain locks/settings changes. core.trust DONE. Existing python3 loads the system Security framework; no Python package dependency.
- Native Messages actions remain BLOCKED: unlocked desktop confirmed without VNC; signed Helper AX-trusted, Messages AX -25204 even immediately after authorized user-owned SecurityAgent dismissal. One real MSG-EDIT prerequisite self-send is indeterminate (-1712); Edit was not dispatched. Normal Quit also times out; sample waits in IMCore/TelephonyUtilities. No foreign service restart or additional recipients. Group eligibility remains LIMITED (approved Remy/Ben pair previously SMS; true iMessage/RCS group unproven).
- Tagged self cleanup: unbounded authorized scan 119 candidates, 0 removed, 119 remain; earlier brief count was 63, increase cause unproven. Native target validation is unavailable. Earlier Remy notices not claimed removed.
- 24/24 Messages sim scenarios and CORE-CREDENTIAL-STORE/sim PASS. Fixed check-messages-capture.sh to build MessagesCaptureChecks when only Helper had been built; failing then passing MSG-WATCHER-REARM retained. make lint and make features PASS (22 done, 1 limited, 1 blocked; 176/183 scenario-levels). No new scenario IDs or SPEC edits.
- Native implementation aaa7ec2 and finish changes stay in messages-finish; no merge/push while requested real E2E is red. Exact commands, session probes, cleanup and historical versus fresh action latency evidence: lab/evidence/messages-finish.json.
