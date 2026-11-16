# Connected apps act at once by default

Supersedes the default level in [ADR 0020](0020-agent-levels-and-owner-approval.md).

An app connected through the Mac's Allow window, `bridge connector allow`, the passkey consent page or `bridge agent create` now gets level `full`: its commands and queries run at once. This includes the operations that used to ask on every request whatever the level: screenshots, screen and audio recording, camera photos, `location.current`, `focus.set` and Photos writes, and the Mac power actions, which `full` agents could not use at all before. On first open after the update the gateway changes every existing `approve` agent to `full` once and records `agent_level_defaults: 2` in the credential store, so an owner's later choice of `approve` is kept. The change keeps each agent's revision, so connected apps keep their sessions.

`approve` stays as the owner's opt-in. `bridge agent level <id> approve|full` switches an existing agent, and the passkey consent page offers it. An `approve` agent behaves as before, including asking for every capture, location read and Photos write.

Connecting stays gated: a new app still needs Allow with the four-digit matching code, so finding the Mac's Tailscale address alone gives no access.

The owner accepted the cost on 2026-11-10. An app that is tricked by something it reads, such as a prompt injection in a message or web page, can send a message, delete a note or take a screenshot with nothing to stop it, and this applies to everyone who installs the release. The reason was that a cloud app working away from the Mac could not complete an approval at all, so the default made connected apps unable to write.
