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

`plist::decode` exposes binary, XML and ASCII plist values, including data, dates and UIDs.
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

Mail and Notes readers are pending. Screen Time, Safari, Calls, Voicemail
and Voice Memos have synthetic stream evidence below. No 24-hour shadow evidence
is claimed.

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

Screen Time source fixture boundary: `UsageSource` accepts an explicit
`blockedHistory` provider and exposes its existing one-shot `scan` method.
The oracle calls that method against an isolated directory; it never starts
capture watchers or calls the default-path `UsageHistory.blocked`. The default production
provider remains unchanged. UUID capture metadata and observation timestamps
are supplied as deterministic fixture context at the sink boundary; record
keys/data, ordering, coverage, scope, completeness and live flags compare
without normalization. The lab CLI has no home-directory defaults.

## Screen Time reader

`usage::Reader::scan(root, context, live)` reads only the supplied Biome tree
and `sync/sync.db`. `Context` carries capture metadata, timezone, local-host
name and live power facts. Its optional `HistoryContext` selects an explicit
knowledgeC fixture/store path, hardware identifier and clock; the SQL reader
matches Swift's local-device and interval filtering. No default user path or
framework probe exists in Rust.

A successful `Scan` includes the unchanged estimated coverage, ordered device
then app-day observations, canonical Go JSON data, source generation,
checkpoint, scan identity, scope and complete-scan evidence. Removed app-day
keys are represented by scope completeness, as in Swift. Call `commit(scan)`
only after the sink acknowledges that checkpoint. Failed reads leave the
committed state intact. Two scans prepared from the same checkpoint cannot
both commit.

The lab-only Cargo example `fixture-reader usage DIRECTORY` emits one
observation per JSONL line followed by a checkpoint line. `--frames` emits a
whole checkpoint frame per line; `--interactive` accepts `scan` lines on stdin.
Every invocation requires an explicit fixture root and `context.json`.

Verification inputs live in `parity/usage`: thirteen Swift rollup vectors
cover timezone/DST boundaries, the fall-back daily cap, fractional timestamps,
deduplication, unclosed sessions, power/lock barriers and system UI exclusions.
`compare.py` runs the actual Swift scan and Rust example through five synthetic
checkpoint transitions, and compares each observation Data hash through Go's
actual encoding/json path. Missing local input and unsupported schema must
produce no capture. All databases are generated under temporary fixture roots.

Reproduce Swift tools with `bash parity/usage/build-swift.sh /tmp/ab-usage-tools`
from this crate (or use the script's repository-relative path). Build the Rust
example and `parity/usage/data-bytes.go` on the build host, then run
`python3 parity/usage/compare.py /tmp/ab-usage-tools/usage-oracle
/home/johan/targets/phase6-native/debug/examples/fixture-reader` on the Air.
The Go helper path expected by that script is
`~/targets/phase6-native/fixture-data-bytes` on `ssh build`.

## Native JSON byte encoding

`native_json::record_data` encodes source projections with Swift JSONEncoder's
floating-point token rules, then applies `ab-wire::gojson` HTML-mode record
encoding. It preserves negative zero, Swift's scientific-notation thresholds,
exponent signs/padding and shortest decimal digits. `foundation_data` retains
Foundation's slash escaping for native fingerprints and body blobs.
`description` formats Swift Double interpolation, including integer `.0`.

The codec is checked against 11,825 deterministic IEEE-754 vectors emitted by
Swift, plus control/Unicode/slash string vectors and Double descriptions.
A fractional-font Notes protobuf fixture verifies HTML font sizes retain their
full precision. Screen Time and the fixture JSONL writer use this typed native
encoding path. Other native reader implementations can call the same functions.

Mac build check: the build-host `aarch64-apple-darwin` metadata probe caught
E0599 for `Metadata::st_flags()` with only the Unix MetadataExt trait in scope.
The Messages module now imports macOS MetadataExt under its target cfg. This
is a platform build fix; the separate Messages worker retains reader ownership.
The probe performs no filesystem read or runtime launch.


## Safari fixture reader

`capture::Snapshot` opens only the supplied SQLite path, read-only, in a read
transaction. Schema requirements, row/time limits, duplicate identities and
file/schema replacement must all pass before complete evidence is available.
Generation strings intentionally use a versioned Rust identity; a migration
requires a complete rescan. It must not reuse an opaque Swift generation.

`safari::Reader::poll(directory, kind, context)` reads History.db,
Bookmarks.plist or CloudTabs.db. Context supplies source generation, checkpoint,
scan ID, timestamp, clock and Automation coverage. `commit(pending)` is called
only after Committed. Concurrent stale acknowledgements are rejected. Caller
owns watcher scheduling; unchanged file/WAL inputs skip scans, and history is
rescanned at its next window expiry. Missing or malformed inputs return errors
and cannot establish absence. Database replacement carries prior candidates
until a second complete scan revalidates absence. `cache` / `resume` use the
same identity-only cache shape as Swift. XML and ASCII plists are parsed as
bounded event streams before constructing recursive values.

The lab-only `fixture-reader safari DIRECTORY [--interactive] [--frames]`
requires context.json. Interactive commands are scan, history, bookmarks,
cloud. It emits observation JSONL plus checkpoint records, or whole frames with
--frames, and persists fixture-only acknowledged identities in rust-cache.json.
An unchanged poll reports `{"kind":"idle"}`; failed polls report source state.
Production code never reads that lab cache. This does not register a source.

Build the actual Swift oracle with
`bash rust/crates/ab-native/parity/safari/build-swift.sh OUTPUT SOURCE_SUPPORT`.
Each Swift compile is separately locked with ab-local. The oracle injects an
empty SafariTabs provider: no app access, Automation request or live files.
Run parity/safari/compare.py with that oracle and the remote fixture-reader
binary. All databases are independently generated from the same synthetic
script on both hosts. The harness pauses only its own oracle during mutations;
repeated identical watcher error notices are coalesced for comparison.

Verified: 19 identical capture/state frames and identical Go JSON Data hashes,
including history title/window expiry, binary/XML bookmarks, reading-list
metadata, cloud device fallback, complete deletions, schema/malformed failures,
two-scan database replacement and identity-cache restart. An unchanged poll
emits no capture. Separate race tests reject file replacement during a snapshot
and stale Safari acknowledgements. Real-data comparison remains orchestrator work.

## Calls and Voicemail

`telephony::calls` / `voicemail` accept an explicit database path, acknowledged
`capture::State`, scan context, live flag and row bound. Their pending batches
include scoped aliases, coverage and complete-scan evidence. They preserve the
Swift ordering and numeric mappings. `State::commit` rejects stale/duplicate
acknowledgements; source or collection changes require a fresh State. Scan
failure leaves the committed state untouched. A new database/schema/metadata
UUID generation emits a full scan without absence from the prior generation.

The lab CLI accepts `calls` and `voicemail` with store.db + context.json under
its explicit fixture directory. Interactive input is a row limit (or scan for
the default 100000). Failed scans produce a failed_scan diagnostic without a
checkpoint. `--frames` includes the full capture metadata. Production callers
own watchers, coalescing, credits and checkpoint persistence.

`parity/telephony/build-swift.sh OUTPUT SOURCE_SUPPORT` compiles the unchanged
Swift sources and scan oracle per command under ab-local. Run compare.py with
that oracle and the remote fixture-reader binary. Both hosts independently
generate synthetic SQLite databases and retain a WAL writer. The replay checks
14 stages per source: fields/provider variants, unchanged scan, row-limit and
corrupt-input failures, updates/deletions, metadata UUID and schema changes,
empty reconciliation and physical database replacement. Data bytes pass through
Go's JSON encoder. Opaque machine-specific generations and their scoped aliases
are renamed bijectively per process; their equality/transition sequence must
match (four epochs). No other metadata or observation data is normalized.


## Voice Memos and staged blobs

`voice_memo::Reader::open(staging_directory)` has no default user path.
`scan(database, context, live, max_rows)` returns pending metadata/audio/folder
observations and blob handles; `commit` follows acknowledgement. The caller
owns event scheduling and should watch the database/WAL, top-level m4a files
and `tracked_audio()` (including missing referenced paths). The source asset
cache keys include file identity, size, mtime and ctime. Late assets are
reprojected on their next changed input. No iCloud downloads are requested.

`assets::Store` stages regular local files using no-follow opens, before/after
identity/size/mtime checks, streaming SHA-256, private files, synchronization
and atomic digest publication. Dataless files on macOS are refused. Handles
read only immutable staged copies in bounded 131072-byte chunks. Caller owns
the staging directory lifecycle. Voice Memos uses voice-memo-blob:DIGEST.
`Batch::blob_handles` is omitted when empty. Blob reads do not advance capture
state. References/handles remain valid after the source file changes or is removed.

`voice_memo::transcript`, `embedded_transcript` and `read_transcript` preserve
Swift JSON integer coercion, ordering, MP4 atom nesting/extended lengths and
malformed-input behavior. File parsing seeks over media payloads and only
loads a bounded transcript atom. 29 checked-in vectors include the exact
RecordingsChecks late-transcript payload, malformed/truncated atoms, depth
limits and numeric edge cases. Rust compares complete Go-encoded result bytes.
`parity/voice-memos/regenerate.py DECODER_ORACLE` reruns the unchanged Swift
transcript decoder over the stored inputs.

The fixture CLI source is voice-memos. Interactive scan/row-limit commands
emit capture frames; blob HANDLE OFFSET LENGTH emits BlobChunk or failed_blob.
The actual Swift scan oracle and retained-WAL replay compare 26 responses,
including late audio/transcript, unchanged scans, immutable old handles after
source mutation, bounded ranges, deletion/folder counts, path and symlink
escapes, embedded NUL/invalid UTF-8, malformed/limited scans and three generation epochs. All observation
Data hashes are compared through Go JSON; opaque generations use the same
bijective epoch normalization as Calls. No live source has been enabled.
