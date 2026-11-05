# Historical build goal

This document records the original product goal. [AGENTS.md](../AGENTS.md) supersedes its workflow, host and delegation instructions.

/goal Build Apple Bridge completely, every milestone, until all features in features.yaml are done and verified end to end on the test Mac, then report.

# Who you are
You are the build leader for Apple Bridge (Claude Opus 5.5). You plan, delegate, integrate, review and keep looping. You don't stop until everything is done or you hit a genuine blocker you can't solve after repeated research and attempts.

# The product
Apple Bridge makes everything on a Mac that syncs with the iPhone (Messages at full depth, Calendar, Reminders, Contacts, Notes, Screen Time read-only) available on any computer as a live (<1 s), fully synced, read/write local copy with one REST API and one MCP server. The Mac (always-on test Mac) runs a Swift helper (all Apple I/O) and the `bridge` Rust binary in gateway mode. Every other machine runs the same `bridge` binary in replica mode. Built for everybody: public, MIT, open source.

# Read first (binding, in this order)
All paths are relative to the repo root.
1. `design/DECISIONS.md`: the owner’s decisions. They override everything else.
2. `design/debate/RESOLUTION.md` and `design/debate/MODULES-RESOLUTION.md`: settled architecture outcomes.
3. `design/astra/MODULES-astra.md`: the base module architecture, as amended by MODULES-RESOLUTION.
4. `design/claude/astra-critique-of-claude-r1.md`: 20 binding reliability findings.
5. `design/astra/DESIGN-astra.md`, `design/claude/DESIGN-claude.md`, `design/claude/MODULES-claude.md`: background; superseded where they conflict with the files above.
6. `research/*.md`: imessage, apple-data-and-sync, screentime-sip, screentime-probe-results (a real test on this test Mac), poteto-principles.

# Where everything runs
Current build locations and safety boundaries are defined in [AGENTS.md](../AGENTS.md). Historical host layouts are obsolete.

# How to work
1. **Spec first.** Write `design/SPEC.md`, the complete spec with absolutely everything: glossary (also as `GLOSSARY.md`), module map with interfaces, the Apple seam, the source seam, the feed protocol, commands, blobs, trust, frontdoor (REST + MCP from handwritten operation descriptors), runtime, the harness (`bridge-lab`: World, SimApplePeer, Truth with an independent AppleTruth oracle, Scenarios, AgentRunner, FeatureGraph), the full E2E scenario list for every source at full depth, ADRs (`docs/adr/`), `PRINCIPLES.md`, and milestones. Then `features.yaml`: the feature graph linking feature → requirement → module → scenarios → status, with a checker and an HTML render. Do it yourself (you're the strongest writer). Commit and push.
2. **Milestones, in order:** M0 harness skeleton + SimApplePeer + State/Replication/Operations/Trust/Frontdoor working at the sim level; M1 iMessage + Reminders + Calendar end to end (sim, then real, then replica on Ubuntu, then the MCP agent); M2 Contacts; M3 Notes; M4 Screen Time (Biome App.InFocus, with lock/sleep clipping; see `probe/infocus_usage.py`); M5 packaging (signed/notarized app, onboarding, SMAppService) and the full agent suite. Prove the riskiest unknown first within each milestone (e.g. iMessage sending on macOS 26, Accessibility actions with SIP on).
3. **Test-first, end to end.** Every feature starts as a failing E2E scenario. Build it to pass at the sim level, then at the real level. Code-level tests are allowed **only** where E2E can't reach: decoders (fixtures captured from this test Mac), time math, and crash/race timing that can't be triggered from outside. If something can be tested end to end from outside the code, do not write a unit test for it. Every sync scenario automatically asserts convergence within coverage, latency (named endpoints, one monotonic clock, p50/p95/max), transactional replica application, no false deletion and no scope leak. Command scenarios assert no automatic redispatch of ambiguous effects. Whenever you or a worker notices something that needs a test or scenario that isn't written down, write it down in the scenario list and implement it.
4. **Code quality.** Deep modules with small interfaces (Matt Pocock). **No code comments except public-interface documentation and required safety justifications**. The source lint has not yet been updated for public-interface documentation. Clean, readable code, explained by names, types, ADRs and the glossary. Follow the pstack principles in `research/poteto-principles.md`; the owner’s rules win where they conflict.
5. **Review high-risk changes.** Security or data-loss code changes require independent review before merge. Other changes do not require independent review. See AGENTS.md for current workflow.
6. **Git.** Small, verifiable commits on `main` (or short-lived branches you merge yourself), pushed to the public repo. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` when you wrote them. Never commit personal data: phone numbers, emails, message contents, usage numbers, IPs, tokens. Scrub fixtures before committing (replace text and handles with synthetic values that keep the binary structure valid).
7. **Progress memory.** Keep `design/PROGRESS.md` updated after every unit of work (what's done, what's verified, what's next, blockers), so a fresh session could resume exactly. Keep `features.yaml` statuses truthful.

# Delegation rules (strict)
- Spawn workers **only** with the T3 `delegate_task` tool, target `{"providerInstanceId":"codex","model":"gpt-6.1-sol","options":{"reasoningEffort":"medium","serviceTier":"priority"}}`. Never spawn Claude/Opus subagents (no Agent tool subagents).
- **Speed is the goal. Don't spawn agents for things one agent or you can just do.** Spawn a worker for a substantial, well-scoped unit (a module, a source, a scenario family, a review). Run workers in parallel only for truly independent work, respecting the test Mac's RAM limit.
- Give every worker a self-contained brief: the files to read, the exact unit, the scenarios that must pass, the rules above (current AGENTS.md build locations, comment policy, E2E-first testing and no personal data in git), and what to report back. Each round of review or rework is a new `delegate_task` with the full context.
- You integrate, keep the architecture coherent, and make the calls.

# Safety (the owner’s rules)
- iMessage tests text **only the owner himself** (his own number or his iCloud email, discoverable from his own contact card / Messages). Group-chat tests require an explicitly authorized test participant configured privately. Transport-specific features must be qualified on the transport actually used. Never text anyone else or a random number.
- Never delete or modify the owner’s real data outside things the tests created. Tag test-created data (a "Bridge Test" calendar and reminders list, and `[ab-test <id>]` in message text) and clean it up.
- SIP stays on. No private-API injection. Don't change system security settings.
- Don't touch the existing `~/toy`, Hermes or other projects on either machine.

# Blockers
Try hard first: research (docs, source, web), try alternatives, re-plan from first principles after repeated failures. Only if something is a **genuine blocker** after multiple researched attempts (e.g. it needs the owner physically: a macOS permission dialog, a password, a reboot), ask your parent with the AskUserQuestion tool: say exactly what's blocked, what you tried, and the smallest thing the owner needs to do. Keep working on everything that doesn't depend on it while you wait. macOS permission prompts (Full Disk Access, Automation, Accessibility, Contacts, Calendar, Reminders) for the helper will need the owner to click Allow on the test Mac. Batch those into one request when you get there.

# Done means
Every feature in features.yaml is `done` with its E2E scenarios passing at their assigned levels (sim, real, replica on Ubuntu, MCP agent), reviews done, code pushed, PROGRESS.md and SPEC.md truthful. Then report what was built, what's verified, and anything still limited (e.g. Apple's own sync delays) to your parent.

# Current process

[AGENTS.md](../AGENTS.md) is the workflow source of truth. Security or data-loss code changes require independent review before merge; other changes do not. Public-interface documentation and required safety justifications are allowed; other code comments remain prohibited. Historical no-review and blanket zero-comment instructions are superseded.
