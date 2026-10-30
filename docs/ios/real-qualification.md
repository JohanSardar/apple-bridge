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

## Sensor harness follow-up

The eight sensor scenarios now launch the installed app with `--real-qualification`, hold a persistent `lockf -k` phone lock, and copy back a final result bound to a fresh run UUID. The host removes its prior local copy before polling again; stale and non-final results cannot qualify a run. Results export checks, counts and digests only. See `lab/iphone-real/README.md` for invocation and scope.

The signed app build and signature verification passed. The host regressions prove that all eight routes fail closed without a configured device, and that stale UUIDs, non-final results and an existing copy destination cannot produce a false pass. These host checks do not qualify phone behavior.

The first coordinated phone batch did not establish a passing scenario. Health read, Health write, Motion and Screen Time did not return final results before their deadlines. Location returned a failure before a fix. Capture returned a failure before recording byte counts or digests. An earlier device-context run observed battery and Focus state, but no Wi-Fi information; NFC was explicitly skipped because it requires a physical tag. The initial alarm timeout was affected by the copy-polling defect and is inconclusive; its corrected rerun returned error code 1 without a confirmed alarm lifecycle.

Permission instructions were printed for Health read, Health read/write, Motion & Fitness, Alarms, Location/Wi-Fi, Screen Time, Location Always/Precise, and Face ID/camera/microphone. Printing an instruction does not prove a system dialog appeared or that Johan tapped it. No tap has been confirmed by these results. No tagged Health write or alarm lifecycle has yet been confirmed, so cleanup is unqualified rather than claimed successful. Capture retains no media on disk. Feature limits remain in place.

Screen Time currently exercises authorization, a separate tagged shield store and monitor registration/removal. Monitor callback and usage report observations remain explicit failed acceptance checks. Location's probe covers a foreground precise fix and its own tagged geofence registration/removal; it does not qualify background movement. Capture's probe covers fresh Face ID, actual photo/audio bytes and in-memory discard; it does not qualify a signed gateway upload.

Redacted results are in `lab/evidence/real-sensor-qualification-2026-10-30.json`. The final combined build was installed with the exact qualification-source digest recorded in that evidence. Its focused Device run again observed battery and Focus, returned no Wi-Fi information, and recorded the NFC physical-tag skip. The corrected alarm rerun returned a failure. An unsuccessful or unavailable interaction remains limited.

The final refreshed Health read rerun also timed out without a final result. All eight sensor scenarios remain limited after these runs. No permission tap was confirmed. The phone lock was released after the last run; no further sensor run or installation remains active.
