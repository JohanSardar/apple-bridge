# ab-native Phase 6 contract

`ab-native` is a network-free library. `make rust-ci` resolves the transitive
Cargo graph and rejects dependencies outside an explicit offline allowlist.
Adding an offline dependency requires reviewing and extending that allowlist.
No Helper launch, TCC request, source registration or live cutover is included.

## Decoder boundary

`typedstream::decode` returns Swift-compatible MessageBody, MessagePart and
MessageFormatting values. Optional fields are omitted from Codable JSON.
Offsets and lengths count UTF-16 code units. Failed decodes retain no text or
parts and include SHA-256 of the original bytes. The parser accepts at most
16 MiB, 64 nested objects/types and 100,000 object references.

`summary::decode` decodes binary `message_summary_info` plists. Edit keys sort
lexically, history order is preserved, retracted parts sort numerically.
Unknown keys or failed nested bodies produce partial status and a raw digest.
Malformed structure produces undecoded status and no edit history.

`plist::decode` exposes binary plist values, including data, dates and UIDs.
`plist::archive` resolves NSKeyedArchive object graphs with cycle, depth and
work limits. No native object instantiation occurs.

`notes::decode` accepts gzip or raw protobuf plus attachment-ID/table-data
mapping. It returns text, ordered attachments, blocks, HTML, rich and lossless
flags. Unknown semantic attributes retain read-only unsupported evidence.
Malformed wire data and invalid UTF-16 run boundaries return typed errors.

`infocus::decode` implements `infocus-segb-1`, including strict ASCII bundle
names and the same finite timestamp bounds. It preserves event order and
duplicates; aggregation belongs to the reader.

## Fixture oracle

`parity/decoders.swift` imports the unchanged Swift Decoders module and reads
only the supplied fixture directories. Checked-in JSONL contains synthetic
and already-scrubbed fixture results, never runtime captures. Rust tests compare
full decoded values and SHA-256 digests, including malformed fixtures and every
synthetic typedstream / basic gzip truncation.

Build the Swift module with `~/bin/ab-local packaging/dev/check-decoders.sh`.
Compile the oracle per command under `~/bin/ab-local`, then pass
`lab/fixtures/messages lab/fixtures/notes lab/fixtures/usage`. Run Cargo and all
other build/test commands on `ssh build` (Ubuntu until its tools are ready).

## Capture ownership

Swift remains the authoritative producer of every source. It assigns capture
IDs, spends credits, receives Committed and persists checkpoints. No decoder
port changes that ownership. A later ab-session integration must explicitly
transfer each responsibility after its separate shadow-comparison gate.

## Pending source work

Screen Time, Safari, Calls/Voicemail/Voice Memos, Mail and Notes readers,
synthetic observation-stream parity and the fixture-reader CLI are not yet
implemented by this decoder slice. No 24-hour shadow evidence is claimed.

## Messages worker coordination (t3/rust-messages-reader)

Messages capture is exclusively owned by t3/rust-messages-reader. New files
`src/messages*.rs`, `examples/messages-reader.rs` and `parity/messages*` belong
to that worker. It will add rusqlite + jiff dependencies and export `messages`.
It consumes `typedstream`, `summary`, `rich`, `plist` directly using your
public structs. Please retain this boundary and omit Messages from your reader
implementation. The worker will rebase onto your decoder PR as it lands.

Decoder API coordination: the modules listed above are public and fixture-verified.
`MessageBody` implements Serialize + Deserialize; `MessageSummary` and rich
projections implement Serialize. `body_digest` returns lowercase hex SHA-256.
`plist::Value` has Null, Bool, Integer(i64), Real(f64), Date(reference seconds),
Data(Vec<u8>), String, Uid(usize), Array and Dictionary(BTreeMap) variants.
`rich::balloon`, `sticker` and `chat_properties` return serde_json Values.
Messages capture ownership is unchanged by this merge. Readers should encode
record data through `ab_wire::gojson::record_data(..., EscapeMode::Html)`.

## Messages capture API

`messages::Reader::open(database, synthetic_or_user_home)` opens the supplied
path read-only; there is no default protected path. `poll(reconcile)` returns
changed `Batch` values with a generation, live/bulk priority and a complete
marker. Call with `false` for changes, and `true` for complete reconciliation.
An unchanged connection-local data version skips all row reads, decodes and
hashes, including complete requests already satisfied for that version. The
caller owns watcher coalescing (100 ms), fallback polling (2 s), wake and 15 min
reconciliation, IPC credits and Committed/checkpoint persistence. This library
is not registered as the active producer; Swift remains authoritative.

`Observation::bytes` uses gojson HTML mode. Observation timestamps and scan IDs
belong to the session boundary, outside the deterministic data projection.
`work_counts` exposes rows materialized and fingerprints computed for idle
regression evidence. `read_blob` accepts a native attachment GUID and a bounded
range; paths resolve under the configured Attachments directory. It refuses
dataless files on macOS and never requests an iCloud download.

Polls hold a read transaction for a consistent snapshot. A failed scan rolls
back only touched fingerprint entries; it does not copy the history-sized
cache on each change. Absence is emitted only with complete evidence, and a
missing table/identity column cannot establish absence for that collection.
Database/schema replacement reopens the connection and invalidates local data
version checkpoints. Prior keys remain candidates for complete reconciliation.

For session integration, prefer `step(request_reconciliation)` and
`reconciliation_pending()`. Each call checks live changes first, then at most
128 history rows, and releases its SQLite snapshot before returning. A data
version change during a paged scan restarts completeness tracking; no absence
is published from interrupted pages. `poll(true)` is the synchronous fixture
convenience that drains history in one call. New inserts precede small-table
and bounded mutation work; live work precedes bulk pages. The caller should
feed every returned batch before taking another step.

## CPU finding from the Swift implementation

`Messages.emitMessages` sorts every row's scalar/blob field names, builds an
intermediate concatenated Data value, separately hashes every blob and hashes
the whole row before it can suppress an unchanged observation. `live` does this
for the recent 2,000 rows on each changed data version; `reconcile` repeats it
for every historical row. `smallChanges` and `emit` add JSON serialization and
hashing of the projected observations. Reopening invalidates the data-version
checkpoint; capture restart also clears the fingerprint caches. Sustained
reconciliation or repeated reopen/restart therefore fits the supplied sample's
String hashing + SQLite hot stacks. This is a static finding, not a new live
profile or proof of which restart trigger was active. The existing Swift
version guard already skips unchanged idle polls.

Rust uses a streaming SHA-256 fingerprint over ordered row fields, cached
prepared statements, only touched-entry rollback journals, and separate native
and observation fingerprints. Idle work counters must remain identical across
21 unchanged polls. Paged reconciliation releases each read transaction and
checks live work before reading another 128 history rows. Session integration
must preserve the connection and these caches rather than reopening per poll.

`resume_candidates(collection, native_keys)` accepts previously committed
identities from session checkpoint storage. They are candidates only: missing
identity columns or an interrupted scan cannot turn them into absence. A
complete scan additionally rechecks missing message GUIDs so a reaction/message
classification change cannot falsely delete a GUID that still exists.

Synthetic replay reproduction:

- On macOS: `bash rust/crates/ab-native/parity/messages-build-swift.sh /tmp/ab-messages-parity`
- Run `python3 rust/crates/ab-native/parity/messages-sequence.py /tmp/ab-messages-parity/messages-oracle lab/fixtures/messages /tmp/messages-swift.json`.
- On a Linux build host: build the `messages-reader` example; run the same
  sequence with its binary and `/tmp/messages-rust.json` as the output path.
  Repeat with `MESSAGES_PAGED=1` for the live-priority paged entry point.
- Copy the Swift result to that host; run
  `go run rust/crates/ab-native/parity/messages-compare.go /tmp/messages-swift.json /tmp/messages-rust.json`.

The oracle compiles the real Swift reader and uses only an explicitly supplied
synthetic database and attachment home. Pins use fixture preferences; dataless metadata is stubbed
in this portable projection oracle. It compares ordered, deduplicated
kind/collection/native_key/data observations after each mutation; Swift JSON
passes through Go's encoder and Rust observations pass through gojson. Session
timestamps, random scan IDs, generation values and asynchronous media
framework derivatives are outside this deterministic reader gate. No active
Helper or native capture registration is exercised by this proof.

Verified synthetic gate: 46 stages, 10,095 observations, zero byte mismatches
against Swift/Go for both synchronous and paged Rust paths. The evidence is
`rust/crates/ab-native/parity/messages-evidence.json`. Full `make rust-ci`
passed, including the workspace rebased onto main. The replay includes binary
pin preferences, SQLite REAL values, cold generations, schema gaps, replacement,
21 unchanged polls, resumed candidates and interrupted history pages.

Two compatibility details established by the oracle: generation changes must
republish unchanged records while retaining previous identities as absence
candidates; a NULL attachment filename currently becomes the process working
directory's last path component in Swift. Rust retains that filename behavior;
the fixture processes use a common synthetic working-directory name.
