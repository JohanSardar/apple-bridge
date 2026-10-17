# MCP agent suite

From the Air worktree, run:

```sh
packaging/dev/heavy.sh go run ./cmd/bridge-lab run lab/scenarios/agent --level agent --json
```

The suite builds an Ubuntu replica and stages signed Helper and AppleTruth apps for each attempt. Ubuntu needs an authenticated `codex` CLI, Python 3, and SSH access from the Air. Tasks whose required features are done run up to three times, stopping after two successes or two failures; every task needs at least two successful attempts. Each attempt uses an isolated Codex home, `gpt-6-luna`, read-only sandboxing, disabled shell tools and one bearer-authenticated MCP server at the Ubuntu replica.

Reminders are checked against AppleTruth for title, list, completion and local due time, then against the replica. Contacts use a fresh, unpredictable fixture job title: the answer must equal AppleTruth and the replica. Agent success claims do not affect the grade. Completed tool events must contain Apple Bridge MCP calls and no other tool calls.

Redacted results go to `lab/evidence/agent/AGENT-SUITE.json`. Raw JSONL and stderr go only to gitignored `lab/evidence/private/<run>/`. Temporary Codex credentials are removed on exit; world cleanup removes remote runtime files. Fixtures carry unique `[ab-test ...]` tags. Reminders, Contacts, and Calendar cleanup is part of the passing grade. An existing Bridge Test list is preserved; a list created by this attempt is removed only after tagged-item cleanup and AppleTruth's unbounded ownership check.

Messages sends select an existing single-participant iMessage chat independently verified against Johan's own handles. A per-attempt Helper policy pins the native chat and exact tagged text, forbids attachments and other commands, and consumes a persistent single-use marker before sending. One send task and at most three attempts enforce a maximum of three outgoing self-texts per suite. Truth reads chat.db read-only, decodes attributed bodies through the system Foundation reader independently of the product, and checks recipient identity, text, and outgoing count; the read task checks a tagged self-message by its Bridge record ID.

Before a send can start, its cleanup manifest is registered under `~/Library/Application Support/AppleBridgeDev/messages-command-cleanup/`, matching the native Messages scenarios. Native IDs, command ID, baseline, and tagged text remain private for later deletion. Evidence distinguishes deferred Messages cleanup from completed cleanup. The suite rejects missing tasks for done sources and multiple Messages send tasks.

Usage selects a historical app-day entry, confirms its estimate against the independent AppleTruth reader over a private snapshot of the Air's Biome files, and asks the agent for its exact estimated seconds through the Ubuntu MCP. Usage values, app names, device identifiers, and answers stay private.
