# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

## [0.1.2] - 2026-11-05

### Added

- Early-access onboarding, contributor and agent guidance, private vulnerability reporting policy, and issue and pull request templates.

### Changed

- Workspace version aligned with the existing 0.1.2 packaging line. This records the source version; it does not announce a public installer.
- Release binaries use fat LTO, one codegen unit and symbol stripping.
- Public interface documentation is allowed; independent review is required for security or data-loss code changes.
- Public documentation uses portable development instructions in place of owner-specific machine setup.
