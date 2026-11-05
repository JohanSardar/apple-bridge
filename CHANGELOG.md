# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- A release Helper trusts only code signed by its own certificate: the gateway socket peer, the bundled `ab-session` and `bridge`, and the parent of `media-facts`. Executable and session overrides now exist only in lab builds. A release gateway must be signed with the Helper's certificate.
- iPhone pairing binds the Mac's signing key: the pairing code commits it and the response is authenticated by the pairing secret.
- Live Activity pushes carry generic text. Real state reaches the app as sealed commands.
- iPhone upload routes verify a signed header before reading the body, with low concurrency limits and a read deadline.
- The Mac checks Certificate Transparency itself and shows a warning in `bridge remote status`, `bridge doctor`, Setup, connector approval and iPhone pairing. This is delayed detection, not prevention.

## [0.1.2] - 2026-11-05

### Added

- Early-access onboarding, contributor and agent guidance, private vulnerability reporting policy, and issue and pull request templates.

### Changed

- Workspace version aligned with the existing 0.1.2 packaging line. This records the source version; it does not announce a public installer.
- Release binaries use fat LTO, one codegen unit and symbol stripping.
- Public interface documentation is allowed; independent review is required for security or data-loss code changes.
- Public documentation uses portable development instructions in place of owner-specific machine setup.
