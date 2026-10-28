# Mail capture

T16 is incomplete. The native reader uses SourceSupport SQLiteSnapshot, CompleteScan and DatabaseWatcher against V10 Envelope Index. A bounded snapshot publishes account and mailbox observations alongside message metadata; an interrupted scan cannot infer absence and a new file generation cannot infer absence against the previous generation. RFC Message-ID strings, when present, provide account-scoped aliases. Unknown fields are null. No body or attachment bytes are captured or replicated.

The fixture contains three mailbox account URLs and 1,000 synthetic messages. Native checks cover metadata, read/move/insert notifications, incomplete scans, generation replacement and loss of access. These checks do not establish gateway delivery or FTS correctness. MAIL-IDLE explicitly fails until idle proof is implemented.

The capture scenario imports native fixture observations through SimApple and checks gateway counts and subject FTS. SimApple preserves native account-scoped aliases.

Remaining work: labels and recipient decoding, sent dates, flags and colors, sizes, snippets, actual local body availability, attachment metadata and on-demand BlobWriter hydration, simapple integration, gateway search/filter and alias reconciliation tests, p95 delivery measurement and five-minute Helper idle CPU evidence. Full Disk Access is required for real Mail data; no real data was read in this work.

No third-party implementation code was copied. Envelope's pinned schema declarations in the research port map informed the read-only SQL joins.

Continuation validation: lint, features and 25 web unit tests pass. Pro Go build/tests passed before the final Ready-frame registration addition; the final refresh and retry failed because the Pro SSH endpoint was unreachable. Native compilation and focused Mail scenarios remained queued on the shared Air and were cancelled without results. Core capture and gateway FTS are therefore implemented but unverified; no PR gate is claimed.

T17 adds an independent Python SQLite Truth reader using a read-only URI, query-only mode and a read transaction against the owner's V10 Envelope Index. It compares the last 30 days by native mailbox identifier and 25 randomly selected subject/from/received-date SHA-256 digests against captured gateway records. Evidence contains counts and digests only. A missing store or fewer than 25 messages fails qualification.

MAIL-REPLICA uses a synthetic metadata-only message and verifies identical records and subject FTS on the gateway and Ubuntu replica. MAIL-AGENT uses a separate synthetic tagged message in a simulated gateway world with an Ubuntu replica; the agent must return the exact subject using MCP only. It never connects to the owner's Mail source. Its transcript is temporary and removed with the world.

This Mac's V10 MailData directory is empty, so MAIL-READ-REAL cannot qualify real capture. MAIL-IDLE at real remains explicitly limited by the missing store and the existing Helper idle CPU issue (T53). Recipient/snippet and attachment limits remain in force.

MAIL-REPLICA passed on Ubuntu. MAIL-AGENT reached its isolated synthetic fixture, but Ubuntu Codex returned HTTP 401 `token_revoked`; login status alone incorrectly reported an existing login. Agent qualification is limited until the owner signs in again. Evidence records exit/tool-call counts and the failure category, with transcripts removed.
