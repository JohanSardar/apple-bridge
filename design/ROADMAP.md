> Historical: kept for reference, not current.

# Apple Bridge roadmap — written 2026-10-27

Everything still to do, in order. The aim is your Mac as a server you sign in to from any device, with every iMCP capability plus as much more of your Apple world as Apple allows, including your iPhone.

**Research this plan rests on:**
- [`research/capability-catalog.md`](../research/capability-catalog.md): 234 capabilities with mechanism, permission, iPhone route, feasibility and value. Row IDs such as `C07` or `F03` below point there.
- [`research/imcp-port-map.md`](../research/imcp-port-map.md): every iMCP tool at `1f3798d`, what to copy and how it fits our Source model.

**Rules carried over from the SPEC:**
- **Done means evidence:** a feature is done only with passing evidence at every level it lists (`features.yaml` status rules). Unprovable work is `limited` with the reason.
- **Writes are commands with evidence:** prepare, dispatch, then observe the effect in Apple's own data.
- **Policy:** SIP stays on. No private-API injection, no TCC edits. Real tests use only the owner's Apple ID, with tagged test data that gets cleaned up.

## Track 0 — this week: prove what exists

| ID | Work | Why | Done when |
|---|---|---|---|
| R0-3 | **Deploy the Screen Time CPU fix (#332)** to the owner's live bridge | The live Helper used 161% of a core | Helper CPU is measured before and after; doctor shows 0 shape violations |
| P0-1 | **iPhone Shortcuts relay spike:** a self-sent iMessage triggers an iPhone automation, which replies with the battery level, then sets Focus and then a Clock alarm, with the phone locked and unlocked | Decides whether Track 4A is possible without an iPhone app (catalog S03) | Pass or fail is recorded per step. If it fails, retry with the email trigger, then fall back to Track 4B |

## Track 1 — remote access through Tailscale

Other devices reach the Mac only on the owner's Tailscale network. TLS ends on the Mac. The route publishes the web app, `/.well-known/*`, `/oauth/*`, `/v1/*` and `/mcp` on one stable tailnet origin, chosen **before** passkey enrollment, because passkeys are bound to the hostname.

| ID | Option | Command on the Mac | What the user does | Other device | TLS ends on Mac | Status |
|---|---|---|---|---|---|---|
| R1-1 | **Tailscale route** | `bridge tailscale` (installed Tailscale app, Tailscale Serve only) | Sign in to Tailscale in the browser it opens, if needed | Signed in to the same tailnet; opens `https://<mac>.<tailnet>.ts.net` | Yes | Needed: install Tailscale when missing, add an owned Serve mapping, and get real evidence from another tailnet device |

**Shared pieces:**
- `bridge tailscale --off`: removes the owned mapping.
- The app shows connection snippets for Claude Code, Codex and Grok for the tailnet URL.

## Track 2 — iMCP parity (port order from the port map)

The new captured Sources follow the Source contract: `open` reports readiness and permissions; `capture` emits records keyed by native keys, with store generations and complete-scan evidence before any deletion; writes use prepare, dispatch and observe. On-demand tools use a new **query contract** (B5) instead of records. iMCP is MIT: copy with the notice kept in a `THIRD_PARTY_NOTICES` file, and pin its dependencies.

| ID | Feature | Catalog rows | Reuse | Kind | Effort |
|---|---|---|---|---|---|
| B1 | **Mail read**: accounts, mailboxes, messages, bodies, attachments | C01–C04, P02 | Envelope 0.5.0 (MIT): Envelope Index reader, MIME parsing, keyset paging | Captured (`mail.account`, `mail.mailbox`, `mail.message`, blobs) | M |
| B2 | **Voice Memos**: recordings, transcripts, audio | M17–M19 | TSRP 0.1.1 (MIT): CloudRecordings.db plus transcript parsing | Captured | M |
| B3 | **Call history and voicemail**: calls, voicemail metadata, transcripts, audio (voicemail on macOS 26+) | C07, C10–C12 | iMCP CallHistory and Voicemail store code | Captured (`phone.call`, `voicemail.message`) | M |
| B4 | **Existing-source upgrades**: Reminders list rename and delete; Contacts Me card; borrow iMCP's date, alarm, recurrence and DST validation fixtures for Calendar and Reminders | X04, X06, X01–X03 | iMCP parsers and fixtures | Existing Sources | S |
| B5 | **Query contract** (new IPC for on-demand tools, with deadlines, cancellation and digest-backed image or audio outputs), then **Location and Maps**: current Mac location, geocoding, nearby places, routes, ETA, map images | L01–L05, L06 | iMCP Location and Maps adapters | On-demand | M, then S–M |
| B8 | **Capture and utilities**: screenshot, camera still, microphone recording, play sound | D12, D14–D16 | iMCP Capture and Utilities | On-demand, with artifacts as blobs | S |
| B9 | **Shortcuts**: list and run with typed input and output. The run result is the evidence; arbitrary effects aren't claimed as verified | S01–S02 | iMCP Shortcuts (the `shortcuts` CLI) | On-demand plus commands | S |
| B10 | **Mail compose (draft) and placing a phone call**, each with honest evidence (draft saved, outgoing call record appears) | C05, C08 | iMCP launch wrappers | Commands | M |

## Track 3 — beyond iMCP, on the Mac

The best value × feasibility rows that iMCP lacks, grouped into features:

| ID | Feature | Catalog rows | Mechanism | Effort |
|---|---|---|---|---|
| C1 | **Mac alarms and timers** | K01–K03, K11 | Shortcuts Clock actions on macOS. Mac alarms don't sync to the iPhone | S (after B9) |
| C2 | **Focus / Do Not Disturb**: read status and current Focus; set Focus, which syncs to the iPhone when "Share across devices" is on | F01–F03, F05 | `INFocusStatusCenter` plus Shortcuts "Set Focus" | S–M; P0-1 verifies phone propagation |
| C3 | **Files and iCloud Drive**: search, read, create, edit, move, trash, download offloaded files, tags | P07–P10, X12 | FileManager, NSFileCoordinator, Spotlight (`NSMetadataQuery`) | M |
| C5 | **OCR and PDF**: text from images, screenshots and scans; search and annotate PDFs | A07, X11 | Vision, PDFKit | S |
| C6 | **Mac controls**: volume and audio devices, Wi-Fi state, network diagnostics, launch and quit apps, clipboard, battery and power, keep awake, lock and sleep, dark mode, Night Shift | D01–D02, D04–D08, D11, D19–D21 | CoreAudio, CoreWLAN, NSWorkspace, NSPasteboard, IOKit, `caffeinate` | S each |
| C7 | **Bridge notifications** on the Mac (agent approvals, alerts) | D17 | UserNotifications | S |
| C8 | **Safari** (removed, ADR 0033): open tabs, iCloud tabs from other devices, history, bookmarks, Reading List, page content | P11–P14, P16–P17 | Safari databases (`History.db`, `CloudTabs.db`, `Bookmarks.plist`) plus AppleScript | M |
| C9 | **Mail actions**: send, reply, forward, organize | C06, P01 | Mail AppleScript; evidence = message appears in Sent or the target mailbox | M |
| C10 | **Audio transcription and an on-device model** | M21, A03 | Speech and Foundation Models frameworks | S each |
| C11 | **Screen Time app and website data** via Apple's new public action, to compare with Biome | F07 | Shortcuts action (macOS 26) | S probe |
| C12 | **Bluetooth devices and battery** | H13–H14 | IOBluetooth | S–M |
| C13 | **Notes extras**: checklists, folders, tags, pins, move | P04–P05, P27 | Existing NotesSource plus AppleScript | M |
| C14 | **iWork, printing, text to speech, music recognition, Time Machine status** | X10, X13–X16 | AppleScript, PDFKit, AVSpeechSynthesizer, ShazamKit, `tmutil` | S each |

## Track 4 — the iPhone

**4A: Shortcuts relay, no iPhone app (if P0-1 passes).** The Mac sends a coded iMessage to the owner. A personal automation (Message trigger, sender = self, Run Immediately) runs one router Shortcut, which replies with the result. The reply is the evidence.

- **Setup:**
  1. Scan a QR code and tap **Get Shortcut** (one router Shortcut does everything; pairing stores the secret automatically).
  2. Follow a 6-tap guide to create the automation (Apple doesn't allow automations to be installed by link).
  3. Tap Run once to grant permissions.
  4. The Mac shows "iPhone connected" when the test reply arrives.
- **Protocol:** each command carries an ID, expiry, secret and replay protection. Requests and replies use separate prefixes. Both messages are deleted afterwards. A timeout means unknown outcome, never a retry.
- **Commands:**

| ID | Command | Catalog rows |
|---|---|---|
| D1 | Clock alarms and timers on the iPhone | K05–K06, K09 |
| D2 | Silent mode, Low Power Mode, volume, brightness, flashlight | I01–I02, I12–I14 |
| D3 | Battery level and charging | I03 |
| D4 | Wi-Fi, Bluetooth, cellular data, Airplane Mode, hotspot (warn: cutting connectivity can block the reply) | I05–I10 |
| D5 | Phone location (a basic "where's my phone") | L11 |
| D6 | Focus on the iPhone directly, if C2 sync isn't enough | F03 |
| D7 | Open an app or URL, speak or play a sound, phone clipboard, screenshot | I24–I27 |
| D8 | Appearance, Night Shift, Always-On, rotation lock, silence unknown callers, other supported app settings | I15–I22, I31 |

**4B: iPhone companion app (the long-term answer).** It needs the Apple Developer Program (see E1) and App Store or TestFlight distribution.

| ID | Feature | Catalog rows |
|---|---|---|
| D9 | Alarms and countdowns with AlarmKit (iOS 26), including stop, snooze and repeat. They appear as our app's alarms, not in Clock | K07–K08 |
| D10 | **Health**: steps, heart rate, HRV, sleep, weight, nutrition, workouts, rings, mindfulness, medications, changes | E01–E10 |
| D11 | Phone location, arrival and departure events | L11, L16 |
| D12 | Approve agent actions with Face ID from a notification; device notifications | X21, D17 |
| D13 | App limits and downtime set by our app (FamilyControls; it can't edit Apple's own limits) | F08–F09 |

## Track E — prerequisites and decisions

| ID | Item | Notes |
|---|---|---|
| E2 | **New permissions in onboarding** | Location, Camera, Microphone, Screen Recording, Focus status, Bluetooth. Each is requested only when its feature is turned on, and appears in `sources.status` and `doctor` |
| E3 | **Per-feature on/off switches and agent scopes** | New sources get their own `<source>:read` and `<source>:write` grants, so agents can be given Mail without Contacts, for example |
| E4 | **Real-test safety for new sources** | Tagged fixtures only (for example a tagged test album, a tagged test file folder, tagged mail drafts). Phone tests only on the owner's devices |

## Blocked by Apple (won't build)

- **Health:** no access from the Mac. Only via the iPhone app (D10).
- **Wallet and Passwords:** no full Wallet, Apple Pay credentials or live Passwords vault.
- **Find My:** no public API for devices, AirTags or people.
- **Screen Time:** can't edit Apple's own Screen Time limits and downtime.
- **Siri:** no reading Siri history and no arbitrary Siri control.
- **Other apps:** can't read other apps' notification history, the cellular call log beyond what syncs, or change TCC or security settings.
- **No guaranteed always-on phone control:** Message automations, iCloud sync and background push are all best effort, and results report that honestly.

## Suggested order

1. **Track 0:** R0-3, P0-1.
2. **R1-1:** a one-command Tailscale route.
3. **Shortcuts first:** B9 Shortcuts, then C1 Mac alarms and timers and C2 Focus, all cheap once B9 exists.
4. **Big data sources:** B1 Mail, B3 Calls and voicemail, B2 Voice Memos.
6. **Larger new sources:** C3 Files, C9 Mail actions.
7. **iPhone without an app:** Track 4A, if P0-1 passed.
8. **iPhone app and Home:** E1, then Track 4B and B11 Home.

Each item becomes a `features.yaml` entry with scenarios (`todo`) when its slice starts, and ships as its own PR with sim and real evidence.
