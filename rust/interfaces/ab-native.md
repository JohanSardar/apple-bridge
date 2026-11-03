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
