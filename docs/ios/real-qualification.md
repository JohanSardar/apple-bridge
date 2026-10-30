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
