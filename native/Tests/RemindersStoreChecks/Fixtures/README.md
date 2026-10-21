# Reminders store fixtures

`manual-order-uuid-array.json` preserves the shape and order of
`ZREMINDERIDSMERGEABLEORDERING_V2_JSON` from a newly created, uniquely tagged Bridge Test list on
the Air (macOS 26.7.1, 2026-10-21). The signed Helper created three reminders in
sequence: a control, a timed reminder with an absolute alarm 15 minutes before
due, and one with an absolute alarm one day before due. The native ordering array
matched those three identifiers in creation order. Each identifier was replaced
with the corresponding synthetic UUID ending in 1, 2, or 3. The Helper removed
the list after an unbounded, completed-inclusive ownership check and verified its
absence with a fresh EventKit store.

All three due-date delta alert blobs were null. Section ordering, membership, and
smart-filter blobs were also null. These are not positive fixtures for those
opaque formats; they remain unsupported until a verified format is available.

The decoder accepts this observed array shape only, with canonical, unique UUID
strings and a 16 MiB input bound. The checks cover exact positions, empty arrays,
malformed JSON and UUIDs, duplicates, unknown version envelopes, every fixture
truncation, byte mutations, and 4,096 deterministic arbitrary byte inputs.
A synthetic SQLite test verifies that the actual V2 JSON column reaches the
typed reader unchanged, that list-only ordering changes update already captured
reminders, and that an unknown envelope clears previously supported positions.

A second tagged probe created five reminders: the control plus absolute and
relative alarms at 15 minutes and one day before a timed due date. All five
due-date delta blobs were null. The V2 array again matched creation order
exactly; the Helper verified cleanup and a fresh-store sweep. Neither EventKit
alarm form is a positive fixture for the native early-reminder format.
