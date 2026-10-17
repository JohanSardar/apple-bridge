# MCP agent suite

From the Air worktree, run:

```sh
packaging/dev/heavy.sh go run ./cmd/bridge-lab run lab/scenarios/agent --level agent --json
```

The suite builds an Ubuntu replica and stages signed Helper and AppleTruth apps for each attempt. Ubuntu needs an authenticated `codex` CLI, Python 3, and SSH access from the Air. Tasks whose required features are done run three times; every task needs at least two successful attempts. Each attempt uses an isolated Codex home, `gpt-6-luna`, read-only sandboxing, disabled shell tools and one bearer-authenticated MCP server at the Ubuntu replica.

Reminders are checked against AppleTruth for title, list, completion and local due time, then against the replica. Contacts use a fresh, unpredictable fixture job title: the answer must equal AppleTruth and the replica. Agent success claims do not affect the grade. Completed tool events must contain Apple Bridge MCP calls and no other tool calls.

Redacted results go to `lab/evidence/agent/AGENT-SUITE.json`. Raw JSONL and stderr go only to gitignored `lab/evidence/private/<run>/`. Temporary Codex credentials are removed on exit; world cleanup removes remote runtime files. Test fixtures carry unique `[ab-test ...]` tags and cleanup is part of the passing grade. An existing Bridge Test list is preserved; a list created by this attempt is removed only after tagged-item cleanup and AppleTruth's unbounded ownership check.
