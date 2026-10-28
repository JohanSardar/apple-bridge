# Mail capture

T16 is incomplete. The native reader uses SourceSupport SQLiteSnapshot, CompleteScan and DatabaseWatcher against V10 Envelope Index. A bounded snapshot publishes message metadata; an interrupted scan cannot infer absence and a new file generation cannot infer absence against the previous generation. RFC Message-ID strings, when present, provide account-scoped aliases. Unknown fields are null. No body or attachment bytes are captured or replicated.

The fixture contains three mailbox account URLs and 1,000 synthetic messages. Native checks cover metadata, read/move/insert notifications, incomplete scans, generation replacement and loss of access. These checks do not establish gateway delivery or FTS correctness. MAIL-IDLE explicitly fails until idle proof is implemented.

Remaining work: account and mailbox observations, labels and recipient decoding, sent dates, flags and colors, sizes, snippets, actual local body availability, attachment metadata and on-demand BlobWriter hydration, simapple integration, gateway search/filter and alias reconciliation tests, p95 delivery measurement and five-minute Helper idle CPU evidence. Full Disk Access is required for real Mail data; no real data was read in this work.

No third-party implementation code was copied. Envelope's pinned schema declarations in the research port map informed the read-only SQL joins.
