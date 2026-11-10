# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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
