# Mail capture

T16 is incomplete. The native reader uses SourceSupport SQLiteSnapshot, CompleteScan and DatabaseWatcher against V10 Envelope Index. A bounded snapshot publishes account and mailbox observations alongside message metadata; an interrupted scan cannot infer absence and a new file generation cannot infer absence against the previous generation. RFC Message-ID strings, when present, provide account-scoped aliases. Unknown fields are null. No body or attachment bytes are captured or replicated.

The fixture contains three mailbox account URLs and 1,000 synthetic messages. Native checks cover metadata, read/move/insert notifications, incomplete scans, generation replacement and loss of access. These checks do not establish gateway delivery or FTS correctness. MAIL-IDLE explicitly fails until idle proof is implemented.

The capture scenario imports native fixture observations through SimApple and checks gateway counts and subject FTS. SimApple preserves native account-scoped aliases.

Remaining work: labels and recipient decoding, sent dates, flags and colors, sizes, snippets, actual local body availability, attachment metadata and on-demand BlobWriter hydration, simapple integration, gateway search/filter and alias reconciliation tests, p95 delivery measurement and five-minute Helper idle CPU evidence. Full Disk Access is required for real Mail data; no real data was read in this work.

No third-party implementation code was copied. Envelope's pinned schema declarations in the research port map informed the read-only SQL joins.

Continuation validation: lint, features and 25 web unit tests pass. Pro Go build/tests passed before the final Ready-frame registration addition; the final refresh and retry failed because the Pro SSH endpoint was unreachable. Native compilation and focused Mail scenarios remained queued on the shared Air and were cancelled without results. Core capture and gateway FTS are therefore implemented but unverified; no PR gate is claimed.
