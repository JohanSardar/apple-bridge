# Real iPhone qualification

2026-10-30: one signed development build, one USB installation and one successful device launch on the paired iPhone 13. Existing profiles were reused. The Go runner used the MacOSX26.4 SDK. No simulator run, additional test suite, phone call, message, Health write, capture, alarm or other phone test data was created. No cleanup was needed.

All 16 requested YAML scenarios were invoked with `go run ./cmd/bridge-lab run <yaml> --level real`. The initial shell/build/pair/command invocations incorrectly passed the generic replica scenario without testing the iPhone. A guard now rejects that fallthrough; the same commands were rerun and returned limited. APP-BUILD evidence records the independently observed build/signature/install/launch proof, not a passing automated acceptance handler.

| Scenario | Functional result and remaining requirement |
| --- | --- |
| APP-BUILD-REAL | Pass: signed build, signature verification, USB install and device launch. |
| APP-SHELL-REAL | Limited: launch succeeded; capability settings and diagnostics were not visually inspected. Real UI observation harness missing. |
| APP-PAIR-REAL | Limited: no real pairing handler; generic replica test previously gave a false positive. Local Network/QR consent and device pairing remain unproved. |
| APP-CMD-REAL | Limited: no real command handler; generic replica test previously gave a false positive. Paired phone/APNs/result round trip remains unproved. |
| APP-NOTIFY-REAL | Limited: handler immediately returns; no phone notification authorization request, delivery or interaction observation. |
| APP-HEALTH-REAL | Limited: handler immediately returns; no HealthKit read authorization request or incremental sample observation. |
| APP-HEALTH-WRITE-REAL | Limited: handler immediately returns; no HealthKit write authorization, tagged write or cleanup route exercised. |
| APP-MOTION-REAL | Limited: handler immediately returns; no motion authorization request or real sensor/history observation. |
| APP-ALARM-REAL | Limited: handler immediately returns; no AlarmKit authorization, alarm/timer lifecycle or cleanup exercised. |
| APP-DEVICE-REAL | Limited: handler immediately returns; no device-context observation, Wi-Fi authorization or foreground NFC tag presentation exercised. |
| APP-SCREENTIME-REAL | Limited: handler immediately returns; no Family Controls authorization, shield, monitor callback or usage report observation. |
| APP-LOC-REAL | Limited: handler immediately returns; no Always/Precise authorization request, location or geofence observation. |
| APP-INTENTS-REAL | Limited: handler immediately returns; no saved Ping iPhone Shortcuts workflow or real pong result observed. |
| APP-APPROVE-REAL | Limited: handler immediately returns the notification placeholder; no Face ID command approval observation. |
| APP-LIVE-REAL | Limited: handler immediately returns; no phone Live Activity start/update/end or widget observation. |
| APP-CAPTURE-REAL | Limited: handler immediately returns; no camera/microphone request, Face ID, capture or cleanup exercised. |

No permission prompt was triggered, so there was no pending prompt to wait three minutes for. These are missing request/observation implementations rather than evidence that Johan declined permission. Implementing real pairing, delivery and permission qualification is larger than a small test-agent fix. Feature limits remain for all unqualified behavior; the build feature already had no limit to remove.

## Installed-app shell and delivery harness qualification

Later on 2026-10-30, the seven shell/delivery scenarios ran against the signed Debug app on the physical iPhone. Each launch used devicectl foreground activation and held the persistent shared phone lock through restoration. The probes use an isolated pinned HTTPS gateway, real development APNs, production command handlers and a separate nonce-bound restoration acknowledgement. Earlier status-route and stale-local-copy fixture failures were fixed before these results. No replica result qualifies these runs.

| Scenario | Result |
| --- | --- |
| APP-SHELL-REAL | Limited: completed native receipt with 21 registered capabilities; capability fields and Diagnostics inspection were not confirmed within three minutes. |
| APP-PAIR-REAL | Pass: real one-time-code pairing and signed status readback; original identity restored. This run used the launch-code path. |
| APP-CMD-REAL | Pass: foreground phone round trip; APNs accepted count 1, callback wake count 1, signed pong observed in the gateway ledger. |
| APP-NOTIFY-REAL | Limited: notification authorization was requested, but no response arrived within three minutes; delivery and button interaction were not reached. |
| APP-APPROVE-REAL | Limited: notification authorization was requested, but no response arrived within three minutes; approval review and Face ID were not reached. |
| APP-LIVE-REAL | Limited: real ActivityKit start, update and end passed; shared widget status readback passed; Lock Screen/widget observation confirmation did not arrive within three minutes. |
| APP-INTENTS-REAL | Limited: zero saved-workflow intent invocations; no pong or workflow-deletion confirmation arrived during the two three-minute phases. |

All seven separate restoration acknowledgements passed. Interrupted notification/approval probes had no completed native receipt; their subsequent restoration launch verified identity, switches, widget baseline and matching tagged resource cleanup. The local Live Activity was ended. No saved Shortcut invocation or deletion was observed, so external Shortcuts cleanup is unconfirmed. Isolated gateway state and private request files were removed; committed evidence contains only scenario identity, checks, counts and digests.

The notification authorization prompt was requested twice, once for notification interaction and once for approval. No Allow tap was observed. Face ID was not reached. Local Network consent was already sufficient for signed pairing; no Local Network tap was confirmed. Shell inspection, Live Activity/widget observation, and saved-Shortcut creation/run/deletion instructions were printed and awaited without a confirming receipt. These absent observations are retained as limits. Only APP-PAIR-REAL and APP-CMD-REAL limits were removed; the command proof covers the requested foreground run.

Source validation passed the signed build, focused Go guard/status-route/receipt-refresh regressions, Go vet, source lint and feature graph. Two sol-loop rounds approved the final behavior after fixing approval-sheet presentation, restore-time widget writes and interrupted tagged cleanup. The later coordinated sensor-only refresh preserved these reviewed harness files; sensor qualification remains owned by its separate workstream.
