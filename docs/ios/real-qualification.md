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

## Sensor harness follow-up

The eight sensor scenarios now launch the installed app with `--real-qualification`, hold a persistent `lockf -k` phone lock, and copy back a final result bound to a fresh run UUID. The host removes its prior local copy before polling again; stale and non-final results cannot qualify a run. Results export checks, counts and digests only. See `lab/iphone-real/README.md` for invocation and scope.

The signed app build and signature verification passed. The host regressions prove that all eight routes fail closed without a configured device, and that stale UUIDs, non-final results and an existing copy destination cannot produce a false pass. These host checks do not qualify phone behavior.

The first coordinated phone batch did not establish a passing scenario. Health read, Health write, Motion and Screen Time did not return final results before their deadlines. Location returned a failure before a fix. Capture returned a failure before recording byte counts or digests. An earlier device-context run observed battery and Focus state, but no Wi-Fi information; NFC was explicitly skipped because it requires a physical tag. The initial alarm timeout was affected by the copy-polling defect and is inconclusive; its corrected rerun returned error code 1 without a confirmed alarm lifecycle.

Permission instructions were printed for Health read, Health read/write, Motion & Fitness, Alarms, Location/Wi-Fi, Screen Time, Location Always/Precise, and Face ID/camera/microphone. Printing an instruction does not prove a system dialog appeared or that Johan tapped it. No tap has been confirmed by these results. No tagged Health write or alarm lifecycle has yet been confirmed, so cleanup is unqualified rather than claimed successful. Capture retains no media on disk. Feature limits remain in place.

Screen Time currently exercises authorization, a separate tagged shield store and monitor registration/removal. Monitor callback and usage report observations remain explicit failed acceptance checks. Location's probe covers a foreground precise fix and its own tagged geofence registration/removal; it does not qualify background movement. Capture's probe covers fresh Face ID, actual photo/audio bytes and in-memory discard; it does not qualify a signed gateway upload.

Redacted results are in `lab/evidence/real-sensor-qualification-2026-10-30.json`. The final combined build was installed with the exact qualification-source digest recorded in that evidence. Its focused Device run again observed battery and Focus, returned no Wi-Fi information, and recorded the NFC physical-tag skip. The corrected alarm rerun returned a failure. An unsuccessful or unavailable interaction remains limited.

The final refreshed Health read rerun also timed out without a final result. All eight sensor scenarios remain limited after these runs. No permission tap was confirmed. The phone lock was released after the last run; no further sensor run or installation remains active.

## I1 qualification on the Pro, 2026-10-31

PR #453's sensor harnesses are combined with the existing shell/delivery harnesses. Sensor and delivery prompt instructions append timestamped plain lines to `~/I1-NEEDS-JOHAN.txt` (`AB_JOHAN_PROMPT_FILE` overrides the destination for tests). Sensor receipts publish nonce-bound prompt phases so the host records each new step and allows its authorization window. Phone locks fail immediately when busy. Sensor qualification is Debug-only and suppresses ordinary foreground capture/upload work while active.

Capture now waits for foreground activation after Face ID and sensor authorization, while retaining expiry and capability checks. Location waits for the Always decision and reads the actual monitored-region set for registration/removal. Alarm qualification preserves the scheduling error if cleanup also fails and records cleanup separately. Their sim harnesses include delayed activation/authorization, cancellation, timeout, successful alarm cleanup and failed schedule/cleanup regressions. Failure evidence uses an error digest rather than a raw error code.

The signed incremental iOS build and signature verification passed. Eleven existing functional sim harnesses passed. The local App Intent dispatcher passed four assertions; extracted metadata covers 29 direct commands and ten Siri shortcuts. The metadata checker now distinguishes direct handlers from deferred notification/capture approval handlers, which cannot run through `CommandIntent`; three checker regressions pass. `APP-SHELL-SIM` still falls through to a generic replica proof, so its reported pass is not shell UI acceptance. There is no separate `APP-INTENTS-SIM` YAML; dispatcher and signed metadata checks provide the local proof.

All fourteen requested real scenarios were attempted sequentially twice on this Pro. The supplied physical iPhone identifier remained unavailable across the reconnect/unlock request and later retries. `devicectl` install and launch both failed to locate the device. The eight sensor scenarios failed `device_launch`; five shell/delivery scenarios failed before copying their request and could not obtain a restoration receipt. No scenario launched on the phone, no authorization tap was observed, and no tagged phone data was created. Temporary host gateway/request directories were removed by the runners. Previously existing phone test data could not be inspected while disconnected.

`FOC-PHONE-SYNC` returned its existing qualification-pending limit without attempting a phone observation. Screen Time callback/report observations and the A1 connector approval prompt are also still unqualified: the existing approval probe exercises command approval, not an OAuth connector approval. No hardware feature limits are removed. Redacted checks, counts and source digests are in `lab/evidence/i1-qualification-2026-10-31.json`.

## Automated physical iPhone follow-up on the Pro, 2026-10-31

This follow-up uses the connected iPhone 13 through devicectl and WebDriverAgentRunner, started with xcodebuild against the physical UDID. WDA HTTP is forwarded through iproxy. All system consent, app buttons, Shortcuts creation/deletion and observation confirmations are automated. Johan performs no taps. No T3 device tools or simulator-control tools are used. The exact paired device is retried every fifteen seconds until it returns; offline time does not consume the native observation window. Instructions go to stderr and optionally `AB_IPHONE_AUTOMATION_STEPS`, never to a request for Johan to interact.

Physical failures identified and fixed HealthKit bulk authorization including per-object types and medication dose events, a background pedometer callback inheriting main-actor isolation, notification completion returning off the main thread, and failure to recognize Screen Time approved-with-data-access authorization. Local regressions cover authorization partitioning, background callback delivery, notification completion ordering and authorization states. The signed incremental build is installed under the shared phone lock.

Health read proved nonempty samples and incremental anchors. Tagged Health writes proved save/read/delete/absence, including cleanup restricted to this app’s source. Alarms proved schedule/readback/cancel/absence. Device context proved battery, charging, Wi-Fi presence and Focus-filter state without exporting names. Location proved a precise foreground fix, Always authorization and asynchronous tagged geofence registration/removal. NFC and background movement remain outside these checks.

The shell proof inspected capability fields and Diagnostics. Notification qualification exercised actual APNs, an automated notification action, a signed result and gateway observation. Saved Shortcuts qualification invoked Apple Bridge’s Check Bridge Status action and observed pong; the automation deleted only `[ab-test] Ping iPhone` and verified no visible tagged workflow remained before confirming cleanup. Live Activity qualification observed the tagged updated activity and the signed widget gallery preview; the activity was ended and shared widget state restored. This does not qualify remote ActivityKit pushes or persistent Home Screen widget installation.

Screen Time authorization, the visible Calculator shield, report rendering, the nonce-bound monitor callback, monitor removal and cleanup pass. Cold attempts initially showed a blank report and were not confirmed. The final fresh foreground run visibly rendered real usage rows and durations before automation confirmed it; only the row count is retained. An interrupted-run marker was recovered. Reports, Downtime & App Limits settings required the existing Screen Time passcode; that prompt was canceled and no setting changed. A speculative layout change did not resolve the cold rendering and was removed; the final proof uses the original report view.

Capture reaches the real Face ID gate; automation cancels it because no non-biometric qualification path exists. No media bytes are captured. Its capability preference is journaled and restored with a nonce-bound acknowledgement. Approval qualification uses the production command and isolated A1 OAuth connector paths. A biometric cancellation is recorded as a failed authentication, and the independent connector phase still runs; no cancellation can qualify an approval or token exchange. Face ID success remains required.

Phone Focus authorization and an inactive baseline pass. The signed Pro helper reports that its Focus-status grant is not determined; no Mac Focus mutation is attempted. The harness does not request a Mac permission dialog. Phone identity, capability preferences, widget state and matching tagged delivery resources are restored and acknowledged separately. The Mac grant and phone transition remain unqualified.

All results remain in the individual redacted evidence files: checks, counts and digests only. Screenshots, personal Health contents, location coordinates, Wi-Fi names, temporary gateway credentials and notification payloads are excluded. APP-SHELL-SIM still uses generic replica convergence and cannot qualify shell UI; APP-SHELL-REAL is the UI proof. There is no APP-INTENTS-SIM YAML, so local dispatcher and signed intent metadata checks provide its separate local coverage. The branch must remain unmerged while any requested real scenario fails.

The final automated result is 11 of 14 real scenarios passed, with capture and both approval paths stopped at Face ID and Focus sync stopped before Mac mutation by the missing Pro helper grant. `lab/evidence/i1d-iphone-pro-2026-10-31.json` binds individual results to source and signed-binary digests. The rebased installed build repeated Device context and the real notification action successfully. Local lint, feature graph, touched Go package tests, fourteen Python tests, twelve existing sim commands, intent dispatcher/metadata checks and native Focus checks passed.
