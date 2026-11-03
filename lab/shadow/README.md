# Shadow IPC tooling

Status: synthetic capture replay and strict database comparison; **not ready for the Phase 4 live shadow gate**. No live gateway has been tapped. Command admission replay and nondeterministic bridge identity alignment remain open.

Run from the repository root with Python 3.9+ and a built `bin/bridge-lab`:

```
bridge-lab shadow record --listen /private/shadow/helper.sock --upstream /isolated/gateway/peer.sock --output /private/shadow/recording
bridge-lab shadow replay --recording /private/shadow/recording/ipc.shadow --starting-home /isolated/stopped-home --output /private/shadow/replayed --go-binary bin/bridge --rust-binary /path/to/rust/bridge
bridge-lab shadow diff --left /private/shadow/replayed/go --right /private/shadow/replayed/rust
python3 lab/shadow/prove.py --output /private/shadow/synthetic-proof
```

The orchestrator owns any future live recording. The Helper must explicitly connect to the proxy socket. The tool never renames, replaces or discovers the gateway's socket. Record forwards the original length prefix and frame bytes in both directions, including authentication, Capture, SourceState, commands and blob traffic. It does not authenticate as a second Helper or decode/re-encode forwarded frames. Each transcript line contains an ordered session number, direction, monotonic time and base64 of the complete raw frame. Stopping with SIGINT or SIGTERM closes the tap. Disk write failure fails recording.

Recordings, replay homes and logs contain sensitive data. Output must be outside the checkout, under an owner-only directory; files use mode 0600. They are local files, never evidence artifacts. Do not upload or commit them. Remove them after the orchestrator's retention window. The synthetic proof prints only booleans and frame type names.

Replay requires an offline starting home and takes its singleton lock without writing to it. It creates new Go and Rust homes, copying ordinary files and using SQLite backup for databases, including WAL contents. Symlink inputs are refused. Only the clones are launched, using file credentials, ephemeral loopback ports and explicitly selected IPC sockets. Fresh nonce authentication replaces the recorded handshake; subsequent Helper frames retain their recorded bytes. Gateway replies must have the recorded type; acknowledgment contents must match, and command/query requests must match their raw recording. A missing runtime or divergent conversation fails. Private process logs stay in the output directory.

Diff requires stopped homes, enforces their singleton locks, and opens databases read-only. It compares exact typed cells and schema column metadata in records, journal, journal commit boundaries, aliases, checkpoints, state metadata, ledger commands, dedupe and ledger metadata. BLOB bytes are never parsed or normalized: reordered gojson, sequence/operation changes and whitespace all cause a digest mismatch. Reports contain only fixed table names, SHA-256 digests and equality flags. Exit 0 means equal; exit 1 means a mismatch or inability to compare. Missing tables fail closed.

Known limitations:

- IPC commands alone do not contain the original authenticated API admission. A recording with Prepare/Dispatch requires admission reconstruction before it can fully replay. The current tool stops at unmatched command traffic; it must not report parity for that recording.
- New bridge IDs and wall-clock journal commit times are generated independently. Two Go replays can therefore differ even from the same starting home. Differences remain visible; there is no silent identity or timestamp normalization. Phase 4 needs an agreed deterministic comparison contract before a zero-diff gate can be claimed.
- Recording timestamps are retained but replay follows conversation order without reproducing wall-clock delays. Expiry, clock-jump, reconnect recovery and incomplete recordings require further qualification.
- The CLI currently requires repository files and Python alongside bridge-lab.

The synthetic proof launches only disposable Go gateways and SimApplePeer. It records and commits captures, records Prepare/Prepared/Dispatch/Outcome and SourceState, checks an identical-home comparison, then changes stored gojson bytes and requires diff to fail. This proves the tap and comparator, not Go/Rust gateway parity.
