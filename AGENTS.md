# Working in Apple Bridge

This file is the source of truth for agent workflow. Historical instructions in design notes are background; current user instructions take precedence. There is no separate CLAUDE.md policy.

## Boundaries

- Read PRINCIPLES.md, features.yaml and the relevant docs/adr before changing behavior. Keep feature status tied to observed evidence; an unverified effect is never a pass.
- Rust owns domain policy, state, replication, trust, operations and gateway protocols. Frontdoor translates REST/MCP; it does not own policy.
- Swift is the native Apple boundary: Apple frameworks, permissions and native UI. Do not add SQLite reads or gateway behavior there. `make lint` checks this boundary using native/boundary-allowlist.txt; do not grow the allowlist to bypass it.
- The iPhone app lives in a separate repository; see CONTRIBUTING.md. Keep protocol and bundle identities compatible.
- No code comments except documentation on public interfaces and required safety justifications. Names, types, ADRs and the glossary explain other code. The existing lint is a baseline, not permission to ban public API documentation.
- Security or data-loss code changes need independent review before merge. Other changes do not require independent review. Report the review and relevant verification in the PR.
- Never commit credentials, personal records, private evidence or host configuration. Keep git author identity unchanged. Do not rewrite history.
- Never modify real data except explicitly authorized, tagged test fixtures. Messaging tests require an explicitly authorized recipient. Do not alter system security settings or a running gateway.

## Build and check

Use the Makefile targets documented in CONTRIBUTING.md. At minimum run `make lint`, `make features` and the focused checks affected by the change. `make rust-ci` is the broader Rust gate; `make rust-golden` checks golden contracts. Preserve lab independence (`make rust-lab-independence`). Do not fix unrelated failures under another ticket.

Do not build on a low-memory Mac running the live gateway. For this environment use an authorized remote build host, a unique scratch checkout and a private CARGO_TARGET_DIR. On the Pro use `cargo -j4`; retry a dropped connection. On a Linux host without Rust, install it into your own directory using RUSTUP_HOME and CARGO_HOME. Remove only your own scratch directories when finished. Do not reuse another agent's target directory.

Keep real Apple-data runs on an explicitly authorized macOS host; Linux checks cannot qualify native behavior. Real runs need isolated state and immutable per-world Helper bundles. Never rebuild a running world's bundle. See packaging/dev/build-helper.sh and packaging/dev/build-truth.sh.

## Delivery

Use short-lived branches and Conventional Commit titles. Review the diff for scope and private data before pushing. Run checks against the final tree, then merge a green PR with `gh pr merge --merge --delete-branch`. Record exact failures and unverified limits instead of claiming success.

The current source lint still rejects doc comments. Updating that enforcement requires a separate Rust-source change; the policy above permits public-interface documentation.
