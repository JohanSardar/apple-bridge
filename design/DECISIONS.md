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
