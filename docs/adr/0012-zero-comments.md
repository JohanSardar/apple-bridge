# Code comments

Names, types, the glossary, the spec and ADRs explain implementation code. Documentation on public interfaces and required safety justifications are allowed; other code comments remain prohibited. This supersedes the original blanket zero-comment policy. `make lint` enforces the existing baseline; see [AGENTS.md](../../AGENTS.md) for current policy.

The current source lint still rejects doc comments. Updating that enforcement requires a separate Rust-source change; the policy above permits public-interface documentation.
