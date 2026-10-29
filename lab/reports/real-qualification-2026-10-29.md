# Real Mac qualification — 2026-10-29

Base: `c3ac9963efa1272847ce1ef053a43f9c53d3fc2a`. Evidence is summary only; no personal data or existing self-chat residue was modified.

| State | Before | After |
|---|---:|---:|
| passed | 71 | 76 |
| failed | 33 | 65 |
| missing | 51 | 14 |

Unique scenario invocations: 64. AI-LOCAL retried once after native-build contention exhausted initial world startup. Disabled real paths remain failed; unsupported levels remain missing.

| Scenario | Before | After | Invocation | Remaining requirement or result |
|---|---|---|---|---|
| AI-LOCAL | missing | passed | exit 0 | Passed current real runner assertions |
| AIRPLAY-REAL | missing | failed | exit 1 | Failed assertions: real_qualification_batched_at_end |
| AUD-TRANSCRIBE | missing | passed | exit 0 | Passed current real runner assertions |
| BT-REAL | missing | failed | exit 1 | Failed assertions: real_qualification_batched_at_end |
| CALL-DIAL-REAL | missing | missing | skipped | skipped: prohibited call placement |
| CLK-ALARM-REAL | missing | failed | exit 1 | Import .build/shortcuts/AB Clock.shortcut after building/signing packaging/shortcuts/ab-clock.json; runner currently disables all real Clock dispatch, so implementation/readback qualification is also required |
| CLK-TIMER-REAL | missing | failed | exit 1 | Import .build/shortcuts/AB Clock.shortcut after building/signing packaging/shortcuts/ab-clock.json; runner currently disables all real Clock dispatch, so implementation/readback qualification is also required |
| CONN-CHATGPT | missing | failed | exit 1 | Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden |
| CONN-CLAUDE-AI | missing | failed | exit 1 | Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden |
| CONN-CLAUDE-CODE | missing | failed | exit 1 | Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden |
| CONN-RELAY-CLAUDE | missing | failed | exit 1 | NS delegation absent; trusted control TLS and VM DNS scopes/IAM require operator setup |
| FOC-IDLE | missing | failed | exit 1 | Five-minute Helper CPU qualification remains pending; no idle CPU measurement claimed. |
| FOC-PHONE-SYNC | missing | failed | exit 1 | Johan must have the paired iPhone available and qualify Focus sync after a safe AB Focus set/readback/restore implementation |
| FOC-SET-REAL | missing | failed | exit 1 | AB Focus pack is not supplied: verify Set Focus identifiers/parameters and provide/import packaging/shortcuts/AB Focus.shortcut; then qualify set/readback/restore (syncs to iPhone) |
| FOC-STATUS-REAL | missing | failed | exit 1 | System Settings > Privacy & Security > Focus: authorize AppleBridgeHelper.app (dev.applebridge.helper); real runner currently hard-codes limited and needs a status probe |
| IWORK-REAL | missing | failed | exit 1 | Failed assertions: real_qualification_batched_at_end |
| LOC-CURRENT-REAL | missing | failed | exit 1 | Failed assertions: fresh_mac_fix |
| MAC-NOTIFY-REAL | missing | failed | exit 1 | Code gap: macRealCommands has no MAC-NOTIFY-REAL handler (mac_real_scenario_supported fails); after implementing tagged notification cleanup, System Settings > Notifications > AppleBridgeHelper must allow notifications. |
| MSG-AUDIO-MESSAGE | missing | failed | exit 1 | Real acceptance harness is not implemented; no native effects dispatched |
| MSG-FORMATTING-SEND | missing | failed | exit 1 | Real acceptance harness is not implemented; no native effects dispatched |
| MSG-MUTE | missing | failed | exit 1 | Set APPLE_BRIDGE_PARITY_HELPER_APP to an immutable signed Helper built from the current source |
| MSG-PINS | missing | failed | exit 1 | Set APPLE_BRIDGE_PARITY_HELPER_APP to an immutable signed Helper built from the current source |
| MSG-READ-RECEIPTS | missing | failed | exit 1 | Set APPLE_BRIDGE_PARITY_HELPER_APP to an immutable signed Helper built from the current source |
| MSG-RECENTLY-DELETED | missing | failed | exit 1 | Set APPLE_BRIDGE_PARITY_HELPER_APP to an immutable signed Helper built from the current source |
| MSG-SEND-EFFECT | missing | failed | exit 1 | Real acceptance harness is not implemented; no native effects dispatched |
| MSG-SEND-LATER | missing | failed | exit 1 | Real acceptance harness is not implemented; no native effects dispatched |
| MUS-CATALOG-REAL | missing | passed | exit 0 | Passed current real runner assertions |
| MUS-CONTROL-REAL | missing | failed | exit 1 | Automation: dev.applebridge.helper → com.apple.Music missing; consent not requested |
| MUS-PLAYLIST-REAL | missing | failed | exit 1 | Automation: dev.applebridge.helper → com.apple.Music missing; consent not requested |
| NOTE-EDIT-PARITY | missing | missing | skipped | skipped: NotesParityProbe executes Apple Events without a non-prompting authorization preflight; cleanup qualification is unproven |
| RELAY-REAL | missing | failed | exit 1 | NS delegation absent; trusted control TLS and VM DNS scopes/IAM require operator setup |
| RMT-TS-EMBEDDED | missing | failed | exit 1 | Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden |
| SAF-READINGLIST-REAL | missing | missing | skipped | skipped: runner creates a Reading List entry without cleanup; after cleanup is implemented, System Settings > Privacy & Security > Automation > AppleBridgeHelper > Safari must be enabled |
| SAF-TABS-REAL | missing | missing | skipped | skipped: runner creates a tab and Reading List entries without cleanup; Safari close_tab is deliberately rejected; after cleanup is implemented, System Settings > Privacy & Security > Automation > AppleBridgeHelper > Safari must be enabled |
| SC-ACTIONS | missing | passed | exit 0 | Passed current real runner assertions |
| SC-IDLE | missing | failed | exit 1 | Failed assertions: five_minute_helper_cpu_under_two_percent |
| SHAZAM-REAL | missing | failed | exit 1 | Failed assertions: real_qualification_batched_at_end |
| SPEECH-SAY | missing | passed | exit 0 | Passed current real runner assertions |
| TM-STATUS | missing | failed | exit 1 | Failed assertions: real_qualification_batched_at_end |
| WEB-CON-PARITY | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-CON-WRITE | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-ATTACH-SEND | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-EDIT | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-REACT | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-REAL-READ | missing | failed | exit 1 | Real acceptance harness is not implemented; no native effects dispatched |
| WEB-MSG-REPLY | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-SEND | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-MSG-UNSEND | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-NOTE-PARITY | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WEB-NOTE-WRITE | missing | missing | exit 1 | Code gap: YAML does not support real (runner rejected requested level) |
| WX-REAL | missing | failed | exit 1 | AB Weather pack is not supplied: verify action identifiers, create/sign/import packaging/shortcuts/AB Weather.shortcut; real runner and structured weather readback are disabled |
| CALL-IDLE | failed | failed | exit 1 | Failed assertions: five_minute_helper_cpu_under_two_percent |
| FILE-OFFLOAD-REAL | failed | failed | exit 1 | Provide a dedicated iCloud-backed [ab-test] file that supports eviction/offload; current temporary fixture could not establish capabilityUnproven. No personal files were used. |
| LOC-IDLE | failed | failed | skipped | Failed assertions: helper_reached_idle, five_minute_idle_cpu_under_2_percent |
| MAC-IDLE | failed | failed | skipped | Failed assertions: helper_cpu_under_two_percent |
| MAIL-BODY-REAL | failed | failed | exit 1 | No V10 Envelope Index on this Mac; real body Truth qualification unavailable |
| MAIL-DRAFT-REAL | failed | failed | exit 1 | grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account |
| MAIL-IDLE | failed | failed | exit 1 | Failed assertions: mail_idle_qualification_pending_T53_and_real_mail_store |
| MAIL-ORGANIZE-REAL | failed | failed | exit 1 | grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account |
| MAIL-READ-REAL | failed | failed | exit 1 | Failed assertions: mail_truth_read_only_available |
| MAIL-SEND-SELF-REAL | failed | failed | exit 1 | grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account |
| MAP-IDLE | failed | failed | skipped | Failed assertions: helper_reached_idle, five_minute_idle_cpu_under_2_percent |
| MSG-DELETE-REAL | failed | failed | skipped | skipped: requires unlocked Mac with Messages UI available and an unused own self-chat handle; no safe UI cleanup demonstrated at lock screen |
| MSG-GROUP-CREATE | failed | failed | skipped | skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario |
| MSG-GROUP-MANAGE | failed | failed | skipped | skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario |
| MSG-READSTATE-REAL | failed | failed | skipped | skipped: requires unlocked Mac with Messages UI available and an unused own self-chat handle; no safe UI cleanup demonstrated at lock screen |
| MSG-RESEND | failed | failed | skipped | skipped: requires a uniquely identified failed self-chat test message via APPLE_BRIDGE_RESEND_RUN; existing self-chat residue must remain untouched |
| MSG-SELFTEST | failed | failed | skipped | skipped: historical selftest leaves text/file residue when UI cleanup is deferred; existing residue untouched |
| MSG-SEND-RCS-GROUP | failed | failed | skipped | skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario |
| MUS-IDLE | failed | failed | skipped | Failed assertions: whole_helper_idle_cpu |
| NOTE-CRUD | failed | failed | skipped | skipped: historical tagged cleanup failed; safe cleanup must be proved before new mutations |
| PHO-IDLE | failed | failed | exit 1 | Photos grant missing; whole-Helper CPU qualification blocked by T53 |
| PHO-IMPORT-REAL | failed | failed | exit 1 | Photos readWrite authorization is not_determined; prompt forbidden |
| PHO-ORIGINAL-REAL | failed | failed | exit 1 | Photos readWrite authorization is not_determined; prompt forbidden |
| PHO-READ-REAL | failed | failed | exit 1 | Photos readWrite authorization is not_determined; prompt forbidden |
| REM-LIST-MANAGE | failed | failed | exit 1 | Code bug: delete_owned_list failed after rename. AppleTruth cleanup guard rejected renamed lists; fixed the one-line guard and verified this run-created list removed (lab/evidence/real-qualification-cleanup.json). Scenario was not retried. |
| RMT-STATUS-OFF | failed | failed | exit 1 | Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden |
| SAF-IDLE | failed | failed | skipped | Failed assertions: signed_helper_world |
| SC-RUN-REAL | failed | failed | exit 1 | Failed assertions: bounded_runner_completed, echo_output_matches |
| SIRI-INTENTS | failed | failed | exit 1 | App Intents metadata missing from Helper bundle; full Xcode metadata toolchain required. Johan must verify Apple Bridge status appears in Shortcuts and runs. |
| USE-ACTION-PROBE | failed | failed | exit 1 | Screen Time action output unmeasured; pack setup: packaging/shortcuts/screen-time-probe.md. No shortcuts executed; retain Biome pending owner-qualified pack. |
| VIS-IDLE | failed | failed | skipped | Failed assertions: helper_cpu_under_two_percent |
| WEB-MEDIA-VARIANTS | failed | failed | skipped | skipped: historical run failed tagged media cleanup; unlock Mac and prove per-run Messages cleanup before new self-chat attachments; existing residue untouched |
| WEB-UPLOAD-NATIVE-STAGING | failed | failed | skipped | skipped: historical run failed tagged media cleanup; unlock Mac and prove per-run Messages cleanup before new self-chat attachments; existing residue untouched |

## Runner gaps and code issues

- Five previously dangling feature references now have explicit failing acceptance placeholders: MSG-AUDIO-MESSAGE, MSG-FORMATTING-SEND, MSG-SEND-EFFECT, MSG-SEND-LATER, WEB-MSG-REAL-READ.
- Web write/parity YAMLs declare only web, despite real feature references. A real harness must be implemented before those levels can qualify.
- Clock, Focus, Weather, Mac extras and remote connector real runners return disabled/limited without exercising their advertised behavior. Imports alone cannot make these pass.
- Safari real writes have no cleanup. Notes parity invokes Apple Events without a non-prompting preflight. Existing Notes CRUD and Messages media/selftest cleanup failures make new writes unsafe in this batch.
- Photos real import/original routes only run PHO-READ-REAL authorization reporting in PhotosChecks; actual import/original assertions are absent. PhotosChecks has a different executable identity from the signed Helper.
- Reminders delete_owned_list still fails after rename. Fixed AppleTruth cleanup guard for the exact renamed tagged title with explicit list ID; verified the run-created list removed. The failed scenario evidence remains unchanged.
- Idle results measure the whole signed Helper. SC-IDLE exceeded its 2% CPU threshold; no passing idle claim is inferred from lower host load.
- SIRI-INTENTS requires App Intents metadata generation and owner verification; its current runner always returns limited.

The AI and audio scenario contracts accept explicit capability/authorization unavailability; their passes do not alone establish available local inference or speech recognition.

## Exact owner setup

| Scenarios | Need |
|---|---|
| MAIL-READ-REAL, MAIL-BODY-REAL, MAIL-IDLE | Configure a dedicated test Mail account with at least 25 messages in the last 30 days and a local V10 Envelope Index; grant the running terminal and signed Helper Full Disk Access under System Settings > Privacy & Security > Full Disk Access. |
| MAIL-DRAFT-REAL, MAIL-ORGANIZE-REAL, MAIL-SEND-SELF-REAL | Configure the test Mail account; System Settings > Privacy & Security > Automation > AppleBridgeHelper > Mail. Real command runner implementation is also required. |
| PHO-READ-REAL, PHO-IMPORT-REAL, PHO-ORIGINAL-REAL, PHO-IDLE | System Settings > Privacy & Security > Photos > AppleBridgeHelper: Full Access; supply a dedicated Bridge Test album with local synthetic assets. Real runner implementation is also required. |
| SC-RUN-REAL | Re-import .build/shortcuts/AB Echo.shortcut compiled and signed from packaging/shortcuts/ab-echo.json. Existing installed AB Echo did not complete or echo the fixture. |
| USE-ACTION-PROBE | No pack exists to import: create AB Screen Time using packaging/shortcuts/screen-time-probe.md, approve Get App & Website Data, export packaging/shortcuts/AB Screen Time.shortcut. |
| LOC-CURRENT-REAL | System Settings > Privacy & Security > Location Services > AppleBridgeHelper: enable location access. |
| MUS-CONTROL-REAL, MUS-PLAYLIST-REAL | System Settings > Privacy & Security > Automation > AppleBridgeHelper > Music; provide a synthetic test playlist and qualify restoration of playback/Recently Played before mutations. |
| MSG-PINS, MSG-MUTE, MSG-READ-RECEIPTS, MSG-RECENTLY-DELETED | Current-source immutable signed Helper via APPLE_BRIDGE_PARITY_HELPER_APP and an unlocked Mac with Messages available; System Settings > Privacy & Security > Accessibility > AppleBridgeHelper; Automation > AppleBridgeHelper > Messages and System Events. Preserve existing self-chat residue. |
| MSG-DELETE-REAL, MSG-READSTATE-REAL, MSG-SELFTEST, WEB-UPLOAD-NATIVE-STAGING, WEB-MEDIA-VARIANTS | Unlock Mac, provide an unused own self-chat handle and prove cleanup of only newly tagged fixtures. Accessibility and Automation grants as above; fix cleanup first. |
| RELAY-REAL, CONN-RELAY-CLAUDE | Delegate the relay DNS zone; provision trusted control TLS and VM DNS scopes/IAM. No production servers were contacted. |
| RMT-TS-EMBEDDED, RMT-STATUS-OFF, CONN-CHATGPT, CONN-CLAUDE-AI, CONN-CLAUDE-CODE | Authorize a dedicated embedded Tailscale node and Funnel interactively; provide the corresponding signed-in connector account and an approved non-production external probe machine. Real runners remain disabled. |

skipped: disruptive — mac.lock_screen, mac.sleep_display, mac.restart, mac.shutdown, mac.set_wallpaper, mac.set_output_device. No separate non-passed real scenario IDs map exclusively to these operations in the target inventory. CALL-DIAL-REAL was never invoked.
