# Johan’s real-run checklist

Generated from every `level: real` limit in `features.yaml` for T99. Grants are for the current signed Helper on the Air. This checklist records prerequisites; it does not authorize prompts, sends, calls, or access to personal data. A grant alone does not qualify a scenario.

## Mac grants

- **MSG-PINS** — limited: needs Accessibility grant for the current signed Helper on the Air. Real preflight submitted zero writes; Automation remains unchecked behind Accessibility. Fresh text-only unused-self-handle fixtures, continuous T63 owned-ID cleanup and baseline preservation are enforced. Pin, receipt and Recently Deleted native qualification remains fail-closed. System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **MSG-MUTE** — limited: needs Accessibility grant for the current signed Helper on the Air. Real preflight submitted zero writes; Automation remains unchecked behind Accessibility. Fresh text-only unused-self-handle fixtures, continuous T63 owned-ID cleanup and baseline preservation are enforced. Pin, receipt and Recently Deleted native qualification remains fail-closed. System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **MSG-READ-RECEIPTS** — limited: needs Accessibility grant for the current signed Helper on the Air. Real preflight submitted zero writes; Automation remains unchecked behind Accessibility. Fresh text-only unused-self-handle fixtures, continuous T63 owned-ID cleanup and baseline preservation are enforced. Pin, receipt and Recently Deleted native qualification remains fail-closed. System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **MSG-SEND-EFFECT** — needs Messages Accessibility grant; signed Helper preflight submitted zero writes and preserved the native baseline System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **MSG-FORMATTING-SEND** — needs Messages Accessibility grant; signed Helper preflight submitted zero writes and preserved the native baseline System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **MSG-RECENTLY-DELETED** — limited: needs Accessibility grant for the current signed Helper on the Air. Real preflight submitted zero writes; Automation remains unchecked behind Accessibility. Fresh text-only unused-self-handle fixtures, continuous T63 owned-ID cleanup and baseline preservation are enforced. Pin, receipt and Recently Deleted native qualification remains fail-closed. System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Messages.
- **WEB-REM-PARITY** — Flag: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Subtasks (indent/outdent): limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Sections (create/move): limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Tags: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — List icon, emoji and group: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Manual reorder: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Images: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-REM-PARITY** — Custom smart lists: limited: Reminders Automation consent pending on this Air (macOS 26.7.1, build 25G241). System Settings > Privacy & Security > Automation > AppleBridgeHelper > Reminders.
- **WEB-CAL-PARITY** — Signed Helper calendar-parity-probe on macOS 26.7.1 (25G241) returned Calendar Apple Events consent pending (-1744), AX cannotComplete (-25204) and zero accessible windows. Exact-event navigation is blocked for invitation Accept/Maybe/Decline, travel time and attachment add/remove; no fixture or effect was attempted and no consent was requested. A subsequent opt-in normal-consent invocation timed out after 60 seconds; the one non-prompting retry after open -g timed out after 30 seconds with no report. No new permission or AX code was returned, and the blocked stage is unverified. Both probe processes were stopped; the already-running Calendar was left open. All controls remain hidden. System Settings > Privacy & Security > Accessibility > AppleBridgeHelper. Also verify Privacy & Security > Automation > AppleBridgeHelper > Calendar.
- **PHO-IDLE** — needs Photos grant System Settings > Privacy & Security > Photos > AppleBridgeHelper.
- **PHO-READ-REAL** — needs Photos grant System Settings > Privacy & Security > Photos > AppleBridgeHelper.
- **PHO-IMPORT-REAL** — needs Photos grant System Settings > Privacy & Security > Photos > AppleBridgeHelper.
- **PHO-ORIGINAL-REAL** — needs Photos grant System Settings > Privacy & Security > Photos > AppleBridgeHelper.
- **MAIL-SEND-SELF-REAL** — grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account System Settings > Privacy & Security > Automation > AppleBridgeHelper > Mail.
- **MAIL-DRAFT-REAL** — grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account System Settings > Privacy & Security > Automation > AppleBridgeHelper > Mail.
- **MAIL-ORGANIZE-REAL** — grant: Automation dev.applebridge.helper → com.apple.mail; no configured Mail account System Settings > Privacy & Security > Automation > AppleBridgeHelper > Mail.
- **CALL-IDLE** — limited: T99 locked Air rerun: needs Full Disk Access for the current signed AppleBridgeHelper on the Air; non-prompting status=denied; capture cannot settle, so no CPU measurement was taken System Settings > Privacy & Security > Full Disk Access > AppleBridgeHelper. Re-run qualification after granting; no idle CPU sample was taken.
- **FOC-SET-REAL** — needs Focus status grant for AppleBridgeHelper; signed Air probe stopped before dispatch; AB Focus import remains unqualified System Settings > Privacy & Security > Focus > AppleBridgeHelper. AB Focus also needs a Shortcuts import for writes.
- **FOC-STATUS-REAL** — needs Focus status grant for AppleBridgeHelper; signed Air probe confirmed the missing grant without requesting authorization System Settings > Privacy & Security > Focus > AppleBridgeHelper. AB Focus also needs a Shortcuts import for writes.
- **MUS-CONTROL-REAL** — needs Music Automation grant (T79); consent not requested System Settings > Privacy & Security > Automation > AppleBridgeHelper > Music.
- **MUS-PLAYLIST-REAL** — needs Music Automation grant (T79); consent not requested System Settings > Privacy & Security > Automation > AppleBridgeHelper > Music.
- **MUS-IDLE** — limited: T99 locked Air rerun: needs Full Disk Access for the current signed AppleBridgeHelper on the Air; non-prompting status=denied; capture cannot settle, so no CPU measurement was taken System Settings > Privacy & Security > Full Disk Access > AppleBridgeHelper. Re-run qualification after granting; no idle CPU sample was taken.
- **MAP-IDLE** — limited: T99 locked Air rerun: needs Full Disk Access for the current signed AppleBridgeHelper on the Air; non-prompting status=denied; capture cannot settle, so no CPU measurement was taken System Settings > Privacy & Security > Full Disk Access > AppleBridgeHelper. Re-run qualification after granting; no idle CPU sample was taken.
- **SAF-TABS-REAL** — limited: needs Safari Automation for dev.applebridge.helper; non-prompting cleanup preflight stopped before fixtures were created. System Settings > Privacy & Security > Automation > AppleBridgeHelper > Safari.
- **SAF-READINGLIST-REAL** — limited: needs Safari Automation for dev.applebridge.helper; non-prompting cleanup preflight stopped before fixtures were created. System Settings > Privacy & Security > Automation > AppleBridgeHelper > Safari.

## Allow tap on the phone

- **APP-HEALTH-REAL** — limited: needs Allow tap
- **APP-HEALTH-WRITE-REAL** — limited: needs Allow tap
- **APP-PAIR-REAL** — limited: needs Allow tap for Local Network and camera QR permission
- **APP-CMD-REAL** — limited: needs Allow tap to complete phone pairing; locked-phone APNs round trip and latency unmeasured
- **APP-NOTIFY-REAL** — limited: needs Allow tap
- **APP-MOTION-REAL** — limited: needs Allow tap
- **APP-ALARM-REAL** — limited: needs Allow tap
- **APP-DEVICE-REAL** — limited: needs Allow tap; Wi-Fi location authorization and foreground NFC tag presentation required
- **APP-SCREENTIME-REAL** — limited: needs Allow tap
- **APP-LOC-REAL** — limited: needs Allow tap for Always and Precise Location
- **APP-INTENTS-REAL** — limited: needs Allow tap to save or import a Shortcuts workflow
- **APP-APPROVE-REAL** — limited: needs Allow tap
- **APP-LIVE-REAL** — limited: needs Allow tap
- **APP-CAPTURE-REAL** — limited: needs Allow tap for camera and microphone

## Shortcuts import or owner verification

- **CLK-ALARM-REAL** — limited: needs import click Use Shortcuts to import/verify AB Clock.
- **CLK-TIMER-REAL** — limited: needs import click Use Shortcuts to import/verify AB Clock.
- **USE-ACTION-PROBE** — Screen Time action output unmeasured; pack setup: packaging/shortcuts/screen-time-probe.md. No shortcuts executed; retain Biome pending owner-qualified pack. Verify the owner-qualified workflow/action in Shortcuts; no import was attempted.
- **SIRI-INTENTS** — Metadata generated with Xcode and bundled; needs Shortcuts listing and status execution verification. Shortcuts CLI lists saved shortcuts, not App Intent actions. Verify the owner-qualified workflow/action in Shortcuts; no import was attempted.

## Account or interactive sign-in

- **RMT-STATUS-OFF** — Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden
- **RMT-TS-EMBEDDED** — Requires an authorized public embedded node and Ubuntu curl; unattended login and system Serve changes are forbidden
- **CONN-CLAUDE-AI** — needs connector login
- **CONN-CHATGPT** — needs connector login
- **CONN-RELAY-CLAUDE** — Bearer-authenticated Claude HTTP connector qualification does not establish interactive OAuth consent
- **MAIL-SEND-SELF-REAL** — Needs a configured test Mail account in addition to the Automation grant. System Settings > Internet Accounts (or Mail > Settings > Accounts).
- **MAIL-DRAFT-REAL** — Needs a configured test Mail account in addition to the Automation grant. System Settings > Internet Accounts (or Mail > Settings > Accounts).
- **MAIL-ORGANIZE-REAL** — Needs a configured test Mail account in addition to the Automation grant. System Settings > Internet Accounts (or Mail > Settings > Accounts).

## Apple approval or service activation

- **CON-NOTE** — CNContactNoteKey read/write requires com.apple.developer.contacts.notes; the signed Helper lacks this restricted entitlement Apple must approve the restricted Contacts notes entitlement.
- **WX-REAL** — limited: T99 locked Air rerun still fails weather.current_completed after Helper readiness, coordinate validation and signing checks pass. WeatherKit activation is not confirmed; the scenario does not retain the response error, so the earlier daemon error is not asserted for this run. WeatherKit activation is not confirmed by a failed request. No System Settings grant fixes a restricted entitlement.

## Engineering, fixtures, or qualification (no permission change is sufficient)

- **MSG-GROUP-CREATE** — skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario
- **MSG-GROUP-MANAGE** — skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario
- **MSG-SEND-RCS-GROUP** — skipped: runner sends to configured non-self Remy/Ben handles; self-chat-only rule forbids this scenario
- **REM-SECTIONS** — EventKit has no sections API; section order and membership decoders remain unverified.
- **NOTE-EXTRAS** — No verified real cleanup route on 25G241; no mutations attempted. Native move and rename require per-build NoteStore evidence and remain unadvertised; sim covers both. Folder deletion requires separate proof. Notes.sdef has no native checklist, hashtag or pin setters; existing decoder reads remain available.
- **NOTE-CRUD** — limited: real append changes native editability and the existing plain-text replace guard rejects the next write; T63 captured-ID cleanup passes with active notes 112 before/after and native objects 120 before/after. No retained residue was selected.
- **WEB-MEDIA-VARIANTS** — limited: needs native attachment cleanup qualification. Scoped recovery could not locate six media rows created by this T63 run; zero foreign rows were deleted. Further real attachment sends are blocked before world creation. This is not a missing-grant limit.
- **WEB-MSG-REAL-READ** — Real acceptance harness is not implemented; no native effects dispatched
- **MSG-SELFTEST** — limited: initial real Helper world startup failed during the changed-module build; no selftest effects were submitted and the independent baseline stayed at 160287 active message/chat identities.
- **MSG-READSTATE-REAL** — skipped: requires unlocked Mac with Messages UI available and an unused own self-chat handle; no safe UI cleanup demonstrated at lock screen
- **MSG-DELETE-REAL** — skipped: requires unlocked Mac with Messages UI available and an unused own self-chat handle; no safe UI cleanup demonstrated at lock screen
- **MSG-RESEND** — skipped: requires a uniquely identified failed self-chat test message via APPLE_BRIDGE_RESEND_RUN; existing self-chat residue must remain untouched
- **WEB-UPLOAD-NATIVE-STAGING** — limited: native send remained indeterminate. T63 recovered this run's three text/file rows with zero foreign deletions. Further attachment sends are blocked pending native cleanup qualification; this is not a missing-grant limit.
- **MSG-SEND-LATER** — needs verified Send Later date controls and cancellation cleanup; fail-closed
- **MSG-MENTION-SEND** — needs self-chat mention suggestion identity and attribute qualification; fail-closed
- **MSG-AUDIO-MESSAGE** — needs native media cleanup qualification; audio remains fail-closed
- **REM-STORE-PERF** — Native warm-full and unchanged modes pass; changed-set modes require native fixtures and remain unproven. Synthetic changed 1/10/100/1000 modes pass.
- **NOTE-EDIT-PARITY** — limited: public Notes HTML does not create native checklist identities and rich editing still needs production-command qualification. The real parity probe and T63 cleanup baseline check pass (120 native objects before/after).
- **RELAY-REAL** — Public relay DNS/TLS and Ubuntu REST/MCP qualified; crt.sh indexing/completeness is not qualified
- **VIS-IDLE** — Failed assertions: helper_cpu_under_two_percent
- **SC-RUN-REAL** — Failed assertions: bounded_runner_completed, echo_output_matches
- **SC-IDLE** — Failed assertions: five_minute_helper_cpu_under_two_percent
- **MAC-IDLE** — Whole Helper measured 2.016546% CPU-time over 300.018 idle seconds after 790.151 seconds of settling under the global real-run lock; still above 2%.
- **MAIL-IDLE** — Failed assertions: mail_idle_qualification_pending_T53_and_real_mail_store
- **MAIL-READ-REAL** — Failed assertions: mail_truth_read_only_available
- **FILE-OFFLOAD-REAL** — Provide a dedicated iCloud-backed [ab-test] file that supports eviction/offload; current temporary fixture could not establish capabilityUnproven. No personal files were used.
- **MAIL-BODY-REAL** — No V10 Envelope Index on this Mac; real body Truth qualification unavailable
- **FOC-IDLE** — Five-minute Helper CPU qualification remains pending; no idle CPU measurement claimed.
- **FOC-PHONE-SYNC** — Johan must have the paired iPhone available and qualify Focus sync after a safe AB Focus set/readback/restore implementation
- **FILE-CRUD-REAL** — CRUD filesystem Truth passes; real cloud download and eviction qualification remains outstanding.
- **MAC-CLIPBOARD-REAL** — Clipboard supports transient text up to 64 KiB; image payloads and approval-required agent writes remain unsupported.
- **MAC-NOTIFY-REAL** — Code gap: macRealCommands has no MAC-NOTIFY-REAL handler (mac_real_scenario_supported fails); after implementing tagged notification cleanup, System Settings > Notifications > AppleBridgeHelper must allow notifications. After the handler/cleanup is implemented: System Settings > Notifications > AppleBridgeHelper > Allow notifications.
- **LOC-IDLE** — Failed assertions: helper_reached_idle, five_minute_idle_cpu_under_2_percent
- **LOC-CURRENT-REAL** — Failed assertions: fresh_mac_fix
- **SAF-IDLE** — Failed assertions: helper_cpu_under_two_percent_for_five_minutes
- **TM-STATUS** — Failed assertions: real_qualification_batched_at_end
- **IWORK-REAL** — Failed assertions: real_qualification_batched_at_end
- **SHAZAM-REAL** — Failed assertions: real_qualification_batched_at_end
- **BT-REAL** — Failed assertions: real_qualification_batched_at_end
- **CALL-DIAL-REAL** — skipped: prohibited call placement

## Additional capability prerequisites without a real scenario entry

- Capture: System Settings > Privacy & Security > Screen & System Audio Recording, Camera, and Microphone > AppleBridgeHelper. The manifest currently records this under CAP-SIM; camera/screen-video implementation gaps also remain.
- APP-WALLET-SIM: Apple approval for FinanceKit, critical alerts, location push, and fall detection; provisioned extensions and a Watch companion also remain prerequisites.
- APP-EXTRAS-SIM: Journaling Suggestions/EnergyKit capabilities, an authorized energy venue, and on-phone selection remain prerequisites.
