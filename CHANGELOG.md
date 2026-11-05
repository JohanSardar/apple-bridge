# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] - 2026-11-05

### Added

- Early-access onboarding, contributor and agent guidance, private vulnerability reporting policy, and issue and pull request templates.

### Changed

- Workspace version aligned with the existing 0.1.2 packaging line. This records the source version; it does not announce a public installer.
- Release binaries use fat LTO, one codegen unit and symbol stripping.
- Public interface documentation is allowed; independent review is required for security or data-loss code changes.
- Public documentation uses portable development instructions in place of owner-specific machine setup.
