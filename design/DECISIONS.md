# User decisions (Johan)

## 2026-10-16, grilling round 1
- **Audience: built for everybody.** A real product other people install, not just Johan's tool.
- **Scope:** all the data that syncs with the iPhone through the Mac: Calendar, Reminders, Messages "and other similar ones" (Contacts, Notes, ...). Mac only: no iPhone app, no iPhone-only APIs.
- **iMessage: every function imaginable, full depth.** Send, edit, react, unsend, reply, and so on. Implication: some of these likely need private APIs. That conflicts with the SIP-on / public-API recommendation and must be resolved explicitly.
- **Speed target: under 1 second** end to end while the Mac is awake, if possible.
- **The Mac (Air) is always on.**
- **Engineering bar: high security.**
- **Next step:** research whether something like this already exists, fully or in parts, before designing further.
- Not yet answered: Q3 history depth (probably "full depth"), Q7 attachments, Q8 calendar range, Q12 name.

## 2026-10-16, grilling round 2 (all accepted)
- Message history: sync what's on the Mac; don't force older iCloud history down in v1.
- Attachments: every replica downloads all of them by default, with a per-machine size cap.
- Calendar window: 2 years back to 2 years ahead, sliding daily.
- Mac is the only data path in v1. A direct-iCloud connector comes later.
- Working name: Apple Bridge.
- Screen Time: read-only source built from Biome App.InFocus (local + synced remote devices). Per app, per device, approximate. SIP stays on. Tested on the Air; see ../research/screentime-probe-results.md.
- iMessage advanced actions: accessibility automation (Beeper platform-imessage approach), SIP on.

## 2026-10-16, grilling round 3 (process and tests)
- **Build everything on the MacBook Air.** The Ubuntu laptop is used when needed as the "other device" (replica plus remote agent) in tests.
- **No separate test account.** Tests use Johan's real Apple account; "just don't do anything stupid". iMessage tests text Johan himself (his number or his iCloud email).
- Accepted: Astra gets the raw requirements plus the settled decisions/research, not Claude's architecture. Two E2E levels (sim + real). A real agent (GPT-6-Luna via Codex, MCP only) plus a scripted client. Code-level tests only where E2E can't reach (decoders, time math, crash/race timing). Test-first from failing E2E scenarios, with the <1 s latency measured. Outputs: GLOSSARY, module map, ADRs, E2E scenario list, features.yaml feature graph.
- **Agents extend the test plan themselves:** whenever an agent finds something that needs a test or an E2E scenario that isn't written down, it writes the scenario down and implements it.

## 2026-10-16, grilling round 4
- No spare Apple IDs. Group-chat tests use the contact "Ben" (number kept in the untracked local test config, never committed) as the other member. NOTE: Johan's thread with Ben shows "Text Message • RCS", i.e. Ben isn't on iMessage. Tests with Ben exercise RCS/SMS groups, not iMessage groups.
- New git repo on the Air plus a GitHub repo (private until public release).
- Johan will fix the Tailscale ACL (ubuntu-server <-> Macs).
- Agents keep building autonomously and never block on Johan.
- **Do not start building until Johan explicitly says so.**

## 2026-10-20, web app grilling (locked)
- **What:** an open-source web UI on top of the bridge covering Messages, Reminders, Calendar, Contacts and Notes. It copies Apple's layout and behavior closely, with polish beyond it. Each user self-hosts on their own Mac or macOS VM. Order: Messages (with Contacts read for names and avatars), then Reminders, Calendar, Contacts, Notes. Screen Time, Photos, Mail and FaceTime are out for now.
- **Shipping:** the frontend lives in this repo and is embedded in the `bridge` binary. Any bridge instance can serve it; users never need Node.
- **Reach from anywhere, free, no client setup:** Tailscale Funnel is embedded through tsnet. `bridge remote on` signs into a free account once and enables Funnel, then prints a stable `https://….ts.net` URL and a QR code. TLS ends on the Mac, so the relay cannot read traffic. Nothing is installed on other devices or agents. A Cloudflare named tunnel is documented as the alternative for people who own a domain. Full copies (replicas) also connect through the relay.
- **Sign-in:** register the first passkey during `bridge setup`. On any computer, sign in by scanning a cross-device passkey QR with your phone.
  - Trusted computer: 30-day session plus the limited cache.
  - Public computer: stores nothing and signs out on close or idle.
  - Every new sign-in alerts your other devices. All sessions are visible and revocable.
  - Strict rate limits, plus one switch in the Mac app that turns remote access off.
- **Agents:** the same URL serves REST and MCP. **Connect an agent** in the web UI issues a scoped token at one of three levels: read-only, approve-first (drafts approved in the UI; preselected) or full. Every agent action goes to an activity log. Copy-paste snippets cover Grok (`mcp(server_url, authorization)`), Claude, Codex, ChatGPT and curl, and OAuth with passkey approval covers connector UIs. The existing localhost token API is unchanged.
- **Writes:**
  - A new "person at the keyboard" path: paired web sessions send to anyone without the test tag, each command tied to a real click or tap.
  - Agent tokens keep their own levels, and today's lab gates stay for tests.
  - `bridge setup` runs a self-test that proves actions on each user's OS build: a tagged send to their own handle, then react, edit, unsend and cleanup.
- **Feature parity:** extend the bridge to close gaps using public frameworks, AppleScript, Accessibility automation and Apple's own databases. Never disable SIP or inject private APIs. Hide what cannot be done. Message types that can't be rendered show a placeholder bubble and are never dropped.
- **Look:**
  - macOS layouts on wide screens and iOS navigation on phones; installable PWA.
  - Liquid Glass only on chrome (toolbars, tab and input bars, sheets, menus); content stays solid. Glass turns off under Reduce Transparency or on weak devices.
  - `system-ui` font with an Inter fallback, and a lookalike line icon set: no SF Pro or SF Symbols shipped.
  - Light and dark follow the system, with a per-device override.
  - No Apple artwork or trademarks in branding. The working name stays; the public name (possibly renaming the project) is decided before public release.
- **Motion:** springs that can always be interrupted, gestures that track the finger 1:1 and shared-element transitions. Polish beyond Apple at four moments: send, reaction, reminder completion and opening media. Reduce Motion is respected.
- **Speed budgets:**
  - Interaction: under 8 ms to first frame.
  - Scrolling: 120 fps through a thread of more than 100k messages, with no layout shift.
  - Live updates: Mac to browser p95 under 500 ms when the gateway serves the UI, or R-LIVE + 100 ms via a replica.
  - Writes are optimistic.
  - Warm load: under 1 s, showing cached data first.
  - Every number is reported with p50, p95 and max.
- **Frontend:** SolidJS + TypeScript + Vite with no component library. Our own spring engine on the Web Animations API, View Transitions and our own virtual list.
- **Data path:**
  - Indexed read models in `state.db`, exposed as ops: `messages.chats` and `messages.thread`, with reactions and edits folded into their messages.
  - One WebSocket per tab for subscriptions, live deltas and command acks.
  - Media over HTTP with immutable caching.
  - The browser keeps a limited IndexedDB cache and a service worker; it is wiped on sign-out or revoke.
- **Media:** the Helper makes browser-safe versions with Apple's hardware encoders:
  - dimensions and a tiny placeholder
  - thumbnail and display sizes, and MP4 with a poster frame
  - a Live Photo still plus a clip, and AAC with waveform data for voice messages
  - transparent stickers
  - originals kept for download
  - new media processed first, then older media backfilled newest to oldest
  - derived files sync as blobs
- **Notifications:** Web Push is off by default and opt-in per device. Notifications carry no message content unless previews are enabled.
- **Quality bar:** macOS keyboard shortcuts, screen-reader labels, Reduce Motion, Reduce Transparency and Increase Contrast. The latest two versions of Safari, Chrome/Edge and Firefox. A string catalog and right-to-left-safe layout, English first.
- **Specs:** `design/web/<app>.md`, written from captures of the real apps on the Air and Apple's Design Resources, lists every action, state and item type with measurements and motion. It is Johan's only stop: he skims each spec before that app is built. Screenshots stay in a gitignored local folder.
- **Demo mode:** `bridge web --demo` serves a rich synthetic dataset built on the sim world. It's used for contributors, README media and CI browser tests.
- **Tests:**
  - Playwright E2E for every spec item on demo/sim data: light and dark, phone and desktop, WebKit, Chromium and Firefox.
  - The same items at the real level on Johan's own Apple ID and handles, tagged and cleaned up.
  - Things that need another person acting (their reactions, edits, typing, iMessage groups) are `limited` at the real level.
  - Automated performance gates on the speed budgets.
  - Code-level tests only for spring and virtual-list math.
  - A new `web` level and `R-WEB-*` requirements in `features.yaml`, and TypeScript added to `make lint`.
- **Process:**
  - Build on the Air.
  - Milestones: W0 Messages study and spec plus design tokens; W1 foundations (front door, Funnel, passkeys, agent connect, shell, demo mode, read models, live path, media pipeline); W2 Messages read; W3 Messages write plus the self-test; W4 Messages parity; then each other app through spec, read, write and parity.
  - Merge on green, following the 2026-10-17 no-review rule.
  - Never ask Johan anything except his spec skims.
- **Install:** a notarized build signed with Johan's Developer ID (Homebrew cask), then `bridge setup` and `bridge remote on`. Setup checks iMessage activation, which is unreliable in macOS VMs (needs macOS 15+ on both host and guest), and says plainly when Messages can't work there.
