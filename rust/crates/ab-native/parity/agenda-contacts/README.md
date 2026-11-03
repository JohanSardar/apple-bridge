# Calendar and Contacts diffing gate

`agenda::Reader` and `contacts::Reader` consume complete observation snapshots
assembled from the signed Helper's native facts. Framework enumeration remains
in Swift. The readers neither open native stores nor register capture sources.
`prepare` returns a pending batch; only an acknowledged batch may be committed.
Persist the reader alongside the acknowledged checkpoint to resume its baseline.

Calendar requires both `calendar.calendar` and `calendar.event` in the snapshot
scope; Contacts requires contact, group and container collections. Duplicate
identities, incomplete snapshots, changed scope and stale acknowledgements fail
without changing the committed state. The caller must validate native paging,
snapshot identity and source generation before constructing this aggregate.

Missing Calendar events require an explicit `Absent`, `Evict` or `Present`
revalidation result. Missing Contacts contacts and groups require explicit
absence confirmation. A full Contacts container scan establishes container
absence. Unconfirmed missing records remain candidates. New generations replay
current records and retain old candidates without emitting false deletions.
Calendar identities replaced by another identity with the same nonempty scoped
alias do not produce a deletion. Tombstones retain the Swift observation data.
Native JSON data bytes use the shared Swift-compatible record encoder.

Run `python3 rust/crates/ab-native/parity/agenda-contacts/regenerate.py` from the
repository root, then `cargo test --manifest-path rust/Cargo.toml -p ab-native
--test agenda_contacts_shadow` with the isolated target required by the run
brief. The generator compiles the exact diff loops extracted from the current
Swift Calendar and Contacts sources, together with the real Calendar time and
scope helpers. It supplies synthetic observations, bounded-window lookup results
and absence confirmations instead of accessing EventKit or Contacts. It never
opens a database, requests a permission or starts the Helper.

The replay compares 9 Calendar and 10 Contacts transitions, including unchanged
polls, data and alias changes, identity replacement, revalidated deletion/window
eviction, forced replay, a Contacts generation reset and empty reconciliation.
It compares complete observations and independently emitted Swift JSONEncoder
data bytes after the shared Go JSON boundary. `evidence.json` records the source
hashes. Separate tests cover incomplete/duplicate/scoped input, stale and foreign
acknowledgements and generation changes without false deletion.

These diff adapters are ready for session integration, not source cutover.
The current native fact catalogue still needs explicit Calendar lookup/window
revalidation and Contacts per-identifier absence confirmation. The session must
also assemble stable aggregate scopes and own capture credits, checkpoint
acknowledgements and persistence. Swift remains selected until those pieces
are wired and the complete routed source is replayed.
