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


## Real Messages follow-up (2026-10-18)
- Corrected the prior VNC-based lock inference. No VNC was used in this attempt. The signed dev Helper is Accessibility-trusted inside Johan's session. The latest GUI probes return Messages AX `-25204`, and process samples show Messages waiting on call capabilities/IMCore queues and `callservicesd` waiting on CoreTelephony. This proves an unresponsive service, not a locked desktop.
- Historical real AppleTruth passes: MSG-REACT `run_01M56MHBV8SHMN009CBF2SJQC2` proved heart and emoji add/remove; MSG-UNSEND `run_01M56MPXB151A0X8AKAZXXPGZ5` proved native retracted-part state even though `date_retracted` remains zero. Each latency has one sample, so p50/p95/max are identical: heart add 11527.44 ms, heart remove 5905.39 ms, emoji add 7698.76 ms, emoji remove 10152.61 ms, unsend 7314.79 ms. Full assertions: `lab/evidence/messages-real-session-green.json`.
- Reply remains unproven: an earlier tagged reply had no `thread_originator_guid`. Edit input/model and semantic-state fixes have not passed a real retest. Group creation remains unproven. The approved Remy/Ben composer offered SMS and rejected an RCS request before sending. SMS does not support the requested membership changes. Own iCloud email acceptance as a distinct participant remains unverified; the Apple minimums are two other participants for rename/add and three for removal. No eligible management success is claimed.
- The login Keychain was confirmed locked before Johan prohibited further tests. The earlier GUI credential attempt caused an authentication dialog. Johan dismissed its user-session SecurityAgent; subsequent user-owned agents were dismissed without entering a password, and the `_securityagent` process was untouched. No further login-Keychain test was run after the prohibition. The lifecycle stays limited; no password or unlock action is requested.
- Cleanup is incomplete: 17 individually tagged self-messages were removed in earlier attempts. The final unbounded candidate audit found 63 active tagged self-messages and the final deletion attempt removed zero (`validate:targetUnavailable`). Expired Remy test notices could not be unsent. The earlier-build empty Bridge Test calendar was independently verified empty, deleted, merged and pushed as `e08fd0fbe1808c1d9a299d45088086caa43aaf14`.
- The final real MSG-REACT and MSG-UNSEND retries both failed at `native_send_observed_with_fingerprint_checkpoint`, before executing the action. The preceding MSG-EDIT retry failed the same prerequisite; its remaining group/reply suite was interrupted and does not establish a new management result. Native implementation and runners are checkpointed on branch `messages-real-session`, commit `aaa7ec2eb8af5c94e2f4d8ed38f618e8140f9480`, and are deliberately not merged because the final real gate is red. This report merges evidence/status corrections only.
- Verification on the worker checkpoint: `packaging/dev/heavy.sh make lint` PASS; `make features` PASS (24 features, 21 done, 3 limited; 175/183 scenario-levels passing). No new scenario IDs, dependencies, or SPEC changes. Exact retry commands and redacted final evidence: `lab/evidence/messages-real-session-final.json`. No implementation feature is newly marked done.


## Messages finish attempt (2026-10-18)
- Login-Keychain lifecycle now PASS without UI: process-local Security interaction is disabled before credential calls; create/read/rotate/revoke takes 302 ms (one sample, p50/p95/max equal). No keychain locks/settings changes. core.trust DONE. Existing python3 loads the system Security framework; no Python package dependency.
- Native Messages actions remain BLOCKED: unlocked desktop confirmed without VNC; signed Helper AX-trusted, Messages AX -25204 even immediately after authorized user-owned SecurityAgent dismissal. One real MSG-EDIT prerequisite self-send is indeterminate (-1712); Edit was not dispatched. Normal Quit also times out; sample waits in IMCore/TelephonyUtilities. No foreign service restart or additional recipients. Group eligibility remains LIMITED (approved Remy/Ben pair previously SMS; true iMessage/RCS group unproven).
- Tagged self cleanup: unbounded authorized scan 119 candidates, 0 removed, 119 remain; earlier brief count was 63, increase cause unproven. Native target validation is unavailable. Earlier Remy notices not claimed removed.
- 24/24 Messages sim scenarios and CORE-CREDENTIAL-STORE/sim PASS. Fixed check-messages-capture.sh to build MessagesCaptureChecks when only Helper had been built; failing then passing MSG-WATCHER-REARM retained. make lint and make features PASS (22 done, 1 limited, 1 blocked; 176/183 scenario-levels). No new scenario IDs or SPEC edits.
- Native implementation aaa7ec2 and finish changes stay in messages-finish; no merge/push while requested real E2E is red. Exact commands, session probes, cleanup and historical versus fresh action latency evidence: lab/evidence/messages-finish.json.


## Messages completion (2026-10-18)
- DONE MSG-REPLY, MSG-EDIT, MSG-REACT and MSG-UNSEND: fresh real AppleTruth passes. Relaunching Messages restored helper Apple events; the direct AX Reply action was replaced with visible context-menu selection to preserve the actual thread relationship. Evidence: `lab/evidence/messages-finish.json` and `lab/evidence/real/MSG-*.json`.
- LIMITED groups: the approved pair offers SMS only; tagged text stayed in the composer, and no group send is claimed. The final Remy notice was unsent and the draft cleared. Apple requires you plus two eligible other iMessage participants. Management eligibility remains limited to the approved contacts.
- Cleanup: 138 messages removed through Messages AX, two tagged group drafts cleared, zero visible tagged messages in independent AppleTruth; four hidden Tapback control rows remain attached to deleted parents under normal Messages retention. All 159,219 untagged baseline rows remain. The 63-to-119 increase is exactly 56 incoming self copies with old timestamps: outgoing stayed 31, incoming rose 32 to 88. Historical ROWID/deletion reconstruction: `lab/evidence/messages-finish-cleanup-growth.json`.
- Test cleanup now executes on every exit for both directions, validates tags and approved peers, clears drafts, persists obligations before sends, and retains unfinished/indeterminate work for reconciliation. Fifty-six historical ledgers are reconciled; the final indeterminate SMS group obligation remains pending despite its current empty audit.
- Corrected the pre-existing signer: it implicitly unlocked the SSH-side development keychain during early builds, contrary to the instruction. No password was created, printed or committed and GUI keychain settings were unchanged. The signer now runs codesign in a bounded one-shot GUI-session job with no password-file read or unlock. Absolute/relative path strict-signature smokes pass. SIP remains enabled.
- Verification: all 120 simulations pass, including 24/24 Messages; Go tests, lint, features and strict signing smoke pass. Fixed the scope probe command envelope and stale store-wide disk-capacity overlays; the quiet 200,000-message benchmark also passes. Features: 23 done, 1 limited, 0 blocked; 180/183 scenario-levels. Initial failures and diagnostics retained. No new scenario IDs or dependencies.

## Web app (DECISIONS 2026-10-20, SPEC §23)
- IN PROGRESS W2-READER: filters, collections, handles, pure rich decoders, native capture and synthetic rich demo merged; independent Truth checks and rich sim checks pass. Real parity is limited because signed AppleTruth lacks Full Disk Access to Messages; Helper counts are supporting evidence. Pin/mute, audio Keep/transcription and recovery deadlines remain explicit coverage gaps.
- DONE W1-DEMO: deterministic 60-contact/40-chat/5,000-message capture world, generated avatars/media, one-use lab demo sign-in, live updates and opt-in 100k chat; WEB-DEMO-DATASET covers all representable items and pre-download geometry with 52 blob hashes, WEB-DEMO-SESSION passes 12/12. Reader gaps and stable native fixture keys are documented in lab/demo/FIXTURES.md.
Process: every change ships as a small PR (WORKER-RULES, Git and GitHub). Status lives in features.yaml; one line per unit here.
- DONE W0: decisions, Messages spec draft (`design/web/messages.md`), SPEC §23 including W2–W8, ADRs 0015–0020, and the drafts for Reminders, Calendar, Contacts and Notes. Johan's skim of the Messages spec is still open. W2 proceeds on the draft and applies his changes when they come.
- DONE W1-RM: chat, thread and collection read models (#2); rich filters and collections, plus `messages.handles` (#14, #22).
- DONE W1-WEB: front door, passkey device sessions and web/1 live views (#3–#5). Hardening: last passkey kept (#15), host pinning (#19), per-IP auth backoff (#23, #24), contact thumbnails (#16), handles route (#38). Pending: local Safari session (#40), and replica upload forwarding (W3).
- DONE W1-UI: Solid shell, design system, springs, virtual list and web-level runner (#6–#8). The 120 Hz and 8 ms targets are unproven on this 60 Hz, headless host.
- IN PROGRESS W1-REMOTE: system Funnel backend real on the Air; public REST/MCP/SSE; REST p95 60 ms through Funnel (#27–#29, #34). Embedded tsnet backend: the real login needs a GUI approval on the Air.
- IN PROGRESS W1-AGENT: credentials (#18), activity (#26), SQLite capture fix (#47), owner approvals (#54) and OAuth Trust grants (#62) merged; OAuth HTTP endpoints pass the sim flow. Agent screens and browser consent follow.
- IN PROGRESS W1-DEMO: generator (#20), demo sessions and CLI (#25), rich runner (#30) finishing exclusive performance reruns.
- IN PROGRESS W1-MEDIA: Helper variants (#9). Real cleanup of six tagged self-chat file bubbles and the cold-arrival latency still need the GUI.
- IN PROGRESS W1-RELAY: local Safari sessions (#40), device-key relay auth (#45), public pairing (#52), feed/runtime (#55) and public replay/revocation proof (#56) merged; Remote command forwarding implemented. Native latency/blob proof and replica agent delegation remain open.
- IN PROGRESS W2: shared primitives (#17), reader decoders (#31 balloons), transcript, list.
- Dev infrastructure:
  - heavy.sh: re-entrant (#11), SDK matching the OS (#35), exclusive lane for browser performance gates (#36, #37).
  - Per-project web budgets (#39); dist placeholder kept after build (#41); detached Air runs (#42).
- **Waiting on the Air being unlocked** (lock screen; never type passwords):
  - Messages capture pass for `messages.md` §17 and the "to verify" rows.
  - Cleanup of the six W1-MEDIA test bubbles.
  - Embedded tsnet login approval (`remote-on.log` in `/tmp/ablab/w1-remote-embedded-check`).

- IN PROGRESS W3-HELPER: interactive origin gate and untagged self-send/cleanup proven; cached per-build self-test proves AppleScript text/file sends; guarded resend, retained-deletion evidence, isolated chat-action proofs and manifest-bound media cleanup implemented. Full UI self-test/read-state/delete proofs, six W1 file bubbles and pending self-test bubble cleanup wait for unlock; real resend requires a safe registered failed self fixture.
