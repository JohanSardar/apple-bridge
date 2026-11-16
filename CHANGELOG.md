# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `bridge mcp` lets any agent on the Mac connect by starting it, with no browser, account or network. The first time each agent connects, the Mac shows the Allow panel once; after that it just works ([ADR 0034](docs/adr/0034-agents-on-this-mac-start-bridge-mcp.md)).
- `bridge link` sets this up in one line for Claude Code, Codex, Cursor, VS Code, Zed, Gemini CLI, Goose, OpenCode and the Claude app. `bridge link other` prints the config for any other agent. The install page has Add to Cursor and Install in VS Code buttons.
- `bridge link grok-bot` explains that Grok Bot runs custom connectors on Grok's cloud computer, so it can't reach Apple Bridge on the Mac.

### Changed

- `bridge link claude` and `bridge link codex` replace their earlier `http://127.0.0.1:7780/mcp` entry with `bridge mcp`. `--no-login` and `--no-open` do nothing now.

### Fixed

- Allow and Don't Allow in the Mac panel failed for about a third of agent requests, whose code contained `-` or `_`.

### Removed

- Safari, Shortcuts and Mac settings are gone: no agent, CLI command, MCP tool or REST route offers Safari history, bookmarks, tabs or Reading List, running or listing shortcuts, or reading and changing Mac settings (battery, display, network, Bluetooth, volume, appearance, apps, clipboard, notifications and the rest). Screen recording stays as part of Capture and now needs `capture:write` instead of `mac:write`, so an agent that can take screenshots can also record the screen. Focus still uses its bundled shortcut. Stored grants and records for these sources are ignored after the upgrade ([ADR 0033](docs/adr/0033-remove-safari-shortcuts-and-mac-settings.md)).
- `make sim` is turned off for now and only prints a message.

## [0.1.7] - 2026-11-12

### Changed

- The project moved from GitHub to GitLab: <https://gitlab.com/johansardar/apple-bridge>. The install command, release downloads, the self-host installer and every link in `bridge` now go through `eilmer.si/applebridge`, so none of them depend on GitHub.

## [0.1.6] - 2026-11-11

### Added

- `photos_show` gives the person the photo itself: full frame and full resolution, nothing drawn on it. Contact sheets are only for finding a photo.
- Setup has a Connect step: local agents use `http://127.0.0.1:7780/mcp`, other devices and Grok Bot connect over Tailscale with `bridge remote enable --via tailnet`, and cloud chats such as Claude use `--via cloud` through Tailscale Funnel.
- `bridge permissions guide` is one small window that points at each macOS prompt or Settings page. Mac approvals need no code.

### Fixed

- Notes writes no longer fail with `permission_missing` when Notes is closed.

### Changed

- MCP answers simple reads in one call. The connect instructions and `apple_read` name the common reads (`reminders.lists`, `reminders.items` with a list name, `notes.list`) and no longer suggest `apple_status` first. `time_zone` is optional wherever the Mac's time zone is filled in. `reminders.lists` returns titles, counts and IDs only. New listed tools: `screen_time` (default yesterday, top five apps per device) and `photos_find` (filters in, a numbered contact sheet out). Message results name the sender and the chat. `apple_describe` ranks reads above capture for looking and finding. Screen Time totals count real rows, whose days carry a time zone and whose seconds are fractional. A request ID belongs to the agent that used it. A read-only agent missing a write scope is told it needs a new agent, and `bridge agents update` refuses write scopes for a read agent instead of storing a read scope. See [ADR 0031](docs/adr/0031-compact-mcp-surface.md).
- MCP is easier for a new agent: `calendar_add` adds events by calendar name, `usage.summary` totals Screen Time per device, reminders can be completed or changed by title, and `apple_describe` finds operations from plain words such as "screen time" or "voicemail". `apple_status` is optional. Names match in any order and without accents, and group chats are found by a participant's name. Local times and dates are accepted, and results show local times. Unknown fields, wrong types and unknown search sources are refused. Text is sent exactly as given. A message that was only partly sent says which part failed and not to repeat it. Search says when it is incomplete. A read outside the agent's scopes says which scope is missing. Repeating a Photos change with the same request ID returns its result. See [ADR 0031](docs/adr/0031-compact-mcp-surface.md).
- MCP clients now see 13 tools instead of 160: `apple_status`, `apple_describe`, `apple_search`, `apple_read`, `apple_change`, `apple_command_status`, `apple_guide`, and `messages_read`, `messages_send`, `reminders_add`, `calendar_events`, `contacts_find` and `notes_create`. The server sends instructions on connect and serves its guides as resources. Changes wait up to 15 seconds and return a short result with the changed record; errors say which field is wrong, what was expected and what to do. Every operation is still reachable, and every old tool name still works, unlisted. See [ADR 0031](docs/adr/0031-compact-mcp-surface.md).
- The `bridge` command follows one set of rules for output, errors, prompts and options; see [CLI changes](docs/reference/cli-changes.md). Usage errors exit 2, Ctrl+C and declined prompts exit 130, and `doctor` exits 1 when it finds problems. Every failure has the same layout, and with `--json` it is one line of JSON on stderr with a stable `reason`. Reads that printed JSON to people now print tables in a terminal; piped, they keep printing the old JSON for one more release with a notice on stderr. Piped `bridge up` prints only the MCP address on stdout. Commands are grouped by task, with plural names such as `bridge agents` and `bridge remote enable`; every old name still works.
- `bridge backup` no longer replaces an existing file without `--force`, and `bridge restore`, `bridge web lock` and revokes show what they change and ask first. Without a terminal they need `--yes`; `agent revoke` and `revoke` still run without it for one release, with a warning.
- Invalid input fails before anything is created, and reads such as `bridge agent list` on a fresh home create no files. A malformed `runtime.json` is reported with its line and column and left as it is.
- `bridge read` tells a source that never synced, or lacks its permission, from one that is empty.
- `bridge agent create` defaults to `--scope reminders:read`; the old default included `system:read`, which agents can't hold, so it always failed.

### Added

- `bridge permissions guide --only NAMES`, `bridge read --next` and `--all`, `bridge call --data`/`--file`/stdin, `--timeout` on `link` and `remote enable`, `bridge help search`, `--help --json`, `bridge help --all --json` and `bridge completion`.

- Connected apps act without asking for approval by default, local or remote, including screenshots, screen and audio recording, camera photos, current location, Focus and Photos changes. Apps connected before this version get the same without reconnecting. Connecting a new app still needs Allow with the matching code. To make an app ask before each change, run `bridge agent level <id> approve`; the passkey consent page also offers it. This trades safety for convenience: an app tricked by something it reads can now send a message or delete a note with nothing to stop it.

## [0.1.5] - 2026-11-08

### Changed

- New hosted connections use `applebridge.eilmer.si`. Existing connections and iPhone pairings retain their identities. Install with `curl -fsSL https://eilmer.si/applebridge/install | sh`.

## [0.1.4] - 2026-11-07

### Added

- Photos: agents can search the library by date, place, kind and album, read originals, and add photos to albums or favorites after approval. Bridge reads PhotoKit live and keeps no copy of the library. Nothing is deleted.

### Fixed

- `bridge permissions request <name>` restarts the running Helper after you allow a permission, as the guide does, so the new grant is used right away. Before, Photos stayed "not allowed" until `bridge down` and `bridge up`.

## [0.1.3] - 2026-11-07

### Changed

- Self-hosted relays run accounts, control and relay as separate system users. Run the installer again to upgrade. To go back, stop control and accounts, run `chown -R ab-relay:ab-relay /var/lib/ab-relay-control /var/lib/ab-accounts`, then run the previous release's installer.

### Security

- A release Helper trusts only code signed by its own certificate: the gateway socket peer, the bundled `ab-session` and `bridge`, and the parent of `media-facts`. Executable and session overrides now exist only in lab builds. A release gateway must be signed with the Helper's certificate.
- iPhone pairing binds the Mac's signing key: the pairing code commits it and the response is authenticated by the pairing secret.
- Live Activity pushes carry generic text. Real state reaches the app as sealed commands.
- iPhone upload routes verify a signed header before reading the body, with low concurrency limits and a read deadline.
- The Mac checks Certificate Transparency itself and shows a warning in `bridge remote status`, `bridge doctor`, Setup, connector approval and iPhone pairing. This is delayed detection, not prevention.
- Abrupt visitor closes, stalled streams and silent connections can no longer use up a Mac's connections, a tunnel or shared relay capacity.
- Login and connector rate limits use the address the relay accepted, with a wider shared budget and no daily lockout when a transport has no source, plus an enrollment link as the owner's way back in.
- Concurrent links can no longer pass an account's Mac limit, retried policy changes apply in order, and account deletion is off until it can revoke the account's Macs.
- The post-login redirect stays on this site, and the link page warns plainly about codes someone else sent.
- Unauthenticated relay routes have per-source budgets, a full nonce pool evicts the oldest entry, and the installer pins the Node.js archive by a checksum verified against its signature.

### Added

- Voicemail records include Apple's transcript and the recording, through the MCP API and `bridge read voicemail`.
- `bridge permissions guide` offers Screen Recording, Camera, Microphone and Location as optional steps. Agents need a fresh owner approval for every capture and current-location read.
- A one-line install: `curl -fsSL https://johansardar.github.io/apple-bridge/install | sh`.
- `bridge up` starts Apple Bridge for agents on this Mac with no account, server, domain or questions. `bridge down` stops it.
- `bridge link claude` and `bridge link codex` connect an agent to the local MCP address, and `bridge unlink` removes it.
- `bridge stats` shows counts per source, never content. `--watch` follows a first sync as each source finishes.
- `bridge read`, `bridge search` and `bridge call` read synced data in a terminal, with `--json` for scripts.
- `bridge permissions guide` and `bridge permissions status` walk through the Mac permissions.
- `bridge skills list` and `bridge skills get` print agent instructions built into the binary.

### Changed

- `bridge` uses a standard argument parser. `--help` and `--version` work, and errors say what went wrong and what to do next. Existing commands and their output are unchanged.

## [0.1.2] - 2026-11-05

### Added

- Early-access onboarding, contributor and agent guidance, private vulnerability reporting policy, and issue and pull request templates.

### Changed

- Workspace version aligned with the existing 0.1.2 packaging line. This records the source version; it does not announce a public installer.
- Release binaries use fat LTO, one codegen unit and symbol stripping.
- Public interface documentation is allowed; independent review is required for security or data-loss code changes.
- Public documentation uses portable development instructions in place of owner-specific machine setup.
