# Voicemail transcripts and audio

`bridge read voicemail` reads `voicemail.message` records. MCP uses `records_query` (the `records.query` operation) with `{"collection":"voicemail.message"}`; both return the same `data` contract, subject to the existing `voicemail:read` authorization. Existing sender, date, duration, read, mailbox and type fields are unchanged.

| Field | Type | Meaning |
| --- | --- | --- |
| `transcript_state` | string | `present`, `missing`, `unavailable` or `unsupported` |
| `transcript` | object, optional | Present only after successful archive decoding |
| `transcript.text` | string | Flattened carrier text, or joined segment text |
| `transcript.segments` | array | Ordered `{text, confidence?}` objects; confidence is a number |
| `transcript.confidence` | number, optional | Overall carrier confidence, when stored |
| `audio_state` | string | `present`, `missing`, `unavailable` or `unsupported` |
| `audio` | object, optional | Original local recording's blob reference |
| `audio.digest` | string | Lowercase SHA-256 of the original recording bytes |
| `audio.size` | integer | Recording length in bytes |
| `audio.state` | string | `present` |
| `audio.mime_type` | string | `audio/amr` for AMR; `audio/mp4` for M4A or audio-only MOV |

`missing` means no transcript bytes or no readable local recording. `unavailable` means the store lacks the relevant optional column/value. `unsupported` means a transcript cannot be decoded or the recording format is not supported. Missing mailbox/provider columns produce `unknown`, not an invented inbox or carrier classification. A transcript can be present without audio, and vice versa. Fields containing unavailable content are omitted, not null.

Download an authorized recording with `GET /v1/blobs/<audio.digest>?record=<record.id>` using the same credentials as the record read. An MCP client can use the gateway's authenticated blob endpoint after `records_query`. Audio bytes are not embedded in record JSON or MCP text. Blob availability follows the normal staging/replication lifecycle; do not treat metadata presence as successful playback or transcoding. AMR support depends on the player's codecs.

Source readiness reports `fields=metadata_and_local_assets`, `transcript=archived_if_present` and `audio=local_if_present`; this describes supported coverage, not a claim that every message has either asset. No permission requests, calls, voicemail mutations or on-demand iCloud downloads are performed.

The machine-readable record shape is `rust/crates/ab-runtime/src/record_shapes.json`. Synthetic archive vectors are in `rust/crates/ab-native/tests/fixtures/voicemail-transcripts.json`. The read-only real-data proof and its limits are documented in [the recording spec](../../design/spec/recordings.md).
