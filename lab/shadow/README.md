# Shadow IPC tooling

The orchestrator owns the later live run. This worker has used only disposable gateways and synthetic data. The tool still fails closed on unsupported conversations; it is not a claim that the full Phase 4 live gate is ready.

From the repository root, with Python 3.9+ and built binaries, the single shell workflow is:

```sh
bridge-lab shadow record --socket-tap "$SHADOW/helper.sock" --upstream "$ISOLATED_SOCKET" --admission-listen 127.0.0.1:18081 --admission-upstream "$ISOLATED_API" --duration 24h --output "$SHADOW/recording" &&
bridge-lab shadow replay --recording "$SHADOW/recording/ipc.shadow" --starting-home "$OFFLINE_STARTING_HOME" --output "$SHADOW/replayed" --go "$GO_BINARY" --rust "$RUST_BINARY" &&
bridge-lab shadow diff --left "$SHADOW/replayed/go" --right "$SHADOW/replayed/rust"
```

`SHADOW` must be an owner-only directory outside every checkout. The starting home must be an offline snapshot from before recording, including credentials for admitted API requests. Replay is Linux-only and requires a network namespace containing only loopback; it attempts an unprivileged `unshare` when needed and never falls back to launching in the host network. Saved nonempty `remote/` adapters are refused because host Unix sockets are not isolated by a network namespace. Helper connections must explicitly use the tap, and API clients must explicitly use the admission listener. The tool never discovers, replaces or renames sockets. An IPC-only recording cannot reconstruct command admission. The API proxy currently supports POST `/v1/op/*` with Content-Length; it is not coverage of MCP, replicated commands, OAuth, approval routes or iPhone ingress.

The admission proxy records the method's operation path, authenticated headers and original request body before forwarding to the configured API listener. Admissions, completed API responses and IPC frames share one ordered transcript. Response boundaries preserve the causal order of commands followed by later captures/source states. Replay submits admissions asynchronously so a waiting API request cannot deadlock the Helper conversation. The original credentials must exist in the starting snapshot. It checks HTTP and operation errors, bounds response waits, and requires recorded Prepare/Dispatch traffic to match. A missing admission or differing command request fails. Command arguments referencing bridge IDs created after the starting snapshot are not rewritten: unsupported identity references fail visibly rather than changing the request hash.

Record forwards original IPC length prefixes and payload bytes, including authentication and unknown payloads. It never authenticates as another Helper or decodes and re-encodes forwarded IPC. Half-closing one direction preserves the response in the other direction. SIGINT/SIGTERM and duration expiry stop recording; disk write failure fails recording. A run ending mid-capture is refused on replay. This proves byte preservation, not absence of added latency or protection from proxy process failure.

```sh
python3 lab/shadow/prove_tap.py --output "$SHADOW/tap-proof"
python3 lab/shadow/prove.py --output "$SHADOW/synthetic-proof"
python3 lab/shadow/prove_isolation.py --output "$SHADOW/isolation-proof"
```

On a Linux build host that disables unprivileged user namespaces, an operator can prepare the replay shell with `sudo unshare --net sh`, enable `lo` with `ip link set lo up`, then drop to the normal build user with `setpriv --reuid <user> --regid <group> --init-groups sh`. Run the workflow or synthetic proof inside that shell. The tool itself never invokes sudo. For the isolation proof only, `--privileged-namespace` explicitly requests a sudo-created namespace and drops privileges before inspecting the fixture. A replay failure retains its private clone/log evidence and never retries outside isolation.

The isolation proof holds a listening sentinel on the host loopback while a child namespace proves that sentinel unreachable, verifies its own loopback is usable, and refuses a saved remote adapter without changing the starting home. This covers IP egress separation; saved adapters require a product-owned offline replay mode before such homes can be accepted.

The tap proof uses fragmented binary and JSON frames, compares every byte received in both directions and every recorded byte, checks half-close responses and duration exit. The synthetic proof starts isolated Go gateways and SimApplePeer, records captures and authenticated command admission through Outcome, requires Go-Go replay parity and seeds record-data, version, journal sequence/op, ledger disposition, expiry, alias, checkpoint and dedupe faults. A data-key-order mutation is required to preserve parsed JSON values while still producing a record mismatch; a timestamp inserted inside record data must also differ, proving that envelope clock exclusions do not scrub source data. It prints only booleans and fixed names.

Replay refuses active recordings using an exclusive-writer/shared-reader file lock, locks the offline starting home, refuses symlinks and creates two new homes with SQLite backup including WAL contents. Only clones run, with file credentials, ephemeral loopback ports and explicit sockets. Fresh nonce authentication replaces the handshake. A read-only SourceState persistence barrier drains final source changes; an IPC Pong alone does not acknowledge them. The final Helper connection stays open until gateway shutdown to avoid adding unrecorded disconnect transactions. Recordings, logs and cloned homes remain local, mode 0600/0700, and must never be committed or uploaded as evidence. Remove them after the orchestrator's retention window.

## Verified synthetic parity

The 2026-11-03 implementation follow-up replays the existing capture-only prefix and a fresh full synthetic command recording against Go and Rust with zero mismatched comparison tables. The full fixture has 24 journal rows and 23 commit boundaries on each side; the prefix has 20 and 19. Rust admission uses the same admin visibility expansion as reads. SourceState preserves Go Coverage field order and null-versus-empty capabilities, avoiding spurious versions and journal transactions when opening Go snapshots. The Go-Go proof and all 14 seeded mismatch checks pass without changing the comparator.

This evidence covers synthetic IPC and REST admission. Deterministic decision clocks, newly allocated IDs embedded in opaque data or arguments, and authenticated observation/replay for MCP, delegated commands, approval decisions and phone ingress remain qualification blockers. The existing offset clock is not a deterministic replay clock. No live traffic was recorded or tapped for this proof.

## Comparison contract

Diff locks stopped homes and opens databases read-only. It compares typed cells and column metadata for records, journal, journal commit boundaries, aliases, checkpoints, state metadata, ledger commands, dedupe and ledger metadata. Missing tables or malformed payloads fail closed. The report contains only fixed table names, SHA-256 digests and equality flags; exit 0 means equal, exit 1 means a mismatch or comparison failure.

Each bridge ID in `aliases.record_id` is assigned a comparison name derived from the sorted set of its `(source, generation, collection, native_key, scoped_alias)` edges. This compares the alias partition, including merges and splits, rather than independently generated ULIDs. Only ID columns and exact JSON string tokens outside `data`, `args` and `evidence` use that map. Unaliased IDs, epochs, admission eras, command IDs and hashes remain exact. A dangling alias produces record/journal mismatches.

Clock exclusions are limited to `records.observed_at`, the record envelope's `observed_at` in records/journal payloads, `journal.committed_at`, and ledger envelope `created_at` and history event `at`. Ledger expiry with an omitted/zero input deadline is compared as an exact nanosecond TTL relative to creation; an explicitly supplied deadline remains exact. Prepare's absolute expiry is excluded from conversation comparison; the ledger TTL/deadline remains the check. No source-data or evidence timestamp is excluded.

JSON is scanned with offsets and only selected scalar tokens are replaced. Other bytes are preserved, including whitespace, field order, escape spellings and number representations. **Record `data` bytes, versions, journal sequence numbers and op names, ledger state, dispositions, per-part results and transition order must match exactly.** Application data containing independently allocated bridge IDs can still differ; this is deliberately unexplained until an implementation-owned deterministic identity seam or a more specific reviewed contract exists. There is no generic JSON reserialization or timestamp scrub.

Remaining live-run qualification includes every admission ingress, overlapping admissions and session replacement, wall-clock expiry/recovery behavior, incomplete recordings, and targets allocated after the starting snapshot. Replay follows recorded conversation order rather than original wall-clock delays and streams the transcript from disk. A 24-hour capacity/retention qualification has not run. The CLI needs repository Python files alongside bridge-lab.

## Recorded decision clocks and all-HTTP ingress

`shadow record` stamps every transcript row with a UTC nanosecond `wall` time and the admission proxy now forwards and records every `GET`/`POST`/`PUT`/`DELETE` under `/v1/` and `/mcp` (REST operations, record and change reads, and MCP tool calls). Each admission row carries `method` and `ingress` (`mcp` for `/mcp`, `rest` otherwise, or `cli`/`feed`/`iphone` when the client sends an `X-Ingress` header). Replay re-issues each row with its recorded method and compares status plus operation state/disposition/error (REST ops) or JSON-RPC id and `isError` (MCP).

```sh
python3 lab/shadow/shadow.py decisions --recording "$SHADOW/recording/ipc.shadow" --home "$FINAL_HOME_COPY" --output "$SHADOW/decisions.json"
python3 lab/shadow/shadow.py replay --recording "$SHADOW/recording/ipc.shadow" --starting-home "$OFFLINE_STARTING_HOME" --output "$SHADOW/replayed" --go "$GO_LAB_BINARY" --rust "$RUST_LAB_BINARY" --decisions "$SHADOW/decisions.json" --exact
```

`decisions` reads `state.db` of a stopped copy of the home read-only for the allocated bridge IDs and sets `clock: recorded`. In that mode replay waits until the gateway's database files are quiet, then moves the lab clock to each row's recorded wall time before delivering it, so decision clocks follow the recorded conversation instead of replay speed. Replay also drains Ready and SourceState persistence before moving on. Go now emits Ready-time source states in sorted order (as Rust already does) so the journal order no longer depends on map iteration.

`prove.py --go "$GO_LAB_BINARY" --rust "$RUST_LAB_BINARY"` records REST, MCP, record-read and `X-Ingress: cli` admissions, then requires the exact decision replay.

Delegated feed commands are covered by the section below. iPhone traffic reaches the gateway over the peer listener, not the HTTP front door, so the recorder does not observe it and replay cannot re-issue it.
Not covered: delegated feed traffic reaches the gateway over the peer listener, not the HTTP front door, so the recorder does not observe it and replay cannot re-issue it (its handshakes use fresh nonces and delegation certificates). A gateway-side feed observer plus a peer-protocol replayer is still required. iPhone ingress is covered below.

## Explicit lab replay decisions

Lab-tagged Go binaries and Rust binaries with `lab-harness` accept `APPLE_BRIDGE_LAB_REPLAY`, pointing at a private JSON manifest. `shadow replay --decisions <manifest> --exact` copies the manifest into each private replay directory and passes it to the cloned gateways. `--exact` compares every typed cell without the alias or envelope-clock exclusions; it does not compare SQLite file pages. The default comparison contract remains unchanged.

The manifest has `now` (an RFC3339 nanosecond timestamp) and `aliases` (a map from `source + NUL + generation + NUL + collection + NUL + native_key` to the recorded bridge ID). State alias allocation consumes these IDs before resolving references into opaque record data. State decision clocks, journal commit clocks and operation admission/approval clocks use the supplied absolute time. Missing alias entries, unreadable manifests and malformed clocks fail closed. Production builds ignore this environment variable. Existing aliases in the starting home retain their persisted IDs.

For the existing disposable synthetic fixture, a focused proof reads its stopped final home to recover the allocated IDs and first command's creation clock, then requires exact replay equality:

```sh
python3 lab/shadow/prove_decisions.py --fixture "$EXISTING_SYNTHETIC_PROOF" --output "$SHADOW/decision-proof" --go "$GO_LAB_BINARY" --rust "$RUST_LAB_BINARY"
```

This is a fixed-clock fixture seam, not a complete recording of clock decisions. It does not yet capture or replay advancing decision clocks, generated command IDs, trust/OAuth clocks or every ingress. The recorder still observes REST command admissions only; authenticated MCP, delegated feed, CLI and iPhone observation/replay remain required before a read-only live capture can be certified. Do not use this fixture manifest to claim a replay of arbitrary recorded live traffic. No live gateway was accessed to develop or verify this seam.

## Gateway-generated command IDs

The decision manifest may carry `command_ids`, an ordered list. In lab replay mode both gateways consume it in order wherever they would otherwise allocate a `cmd_` ID for a command submitted without one (Go `contracts.NewCommandID`, Rust `ab_state::replay::command_id`). An exhausted or malformed queue fails closed. `shadow decisions --starting-home <home>` fills it from the final ledger: commands absent from the starting ledger whose ID never appears in a recorded admission body, ordered by creation time. `prove.py` now submits a REST command without `command_id` and requires 8 of 8 exact Go-vs-Go replays plus zero Go-vs-Rust table differences (`--go-go-runs` sets the count).

Not covered by the queue: IDs allocated for rejected agent commands that never reach the ledger, and `qry_` query IDs. The delegated feed and iPhone observer/replayer blockers above remain open.

## Delegated feed ingress

Replica commands arrive at the gateway as `POST /v1/commands` on the peer listener (mTLS plus an `X-Apple-Bridge-Delegation` Ed25519 header). Go (`LocalGateway.Forward`) and Rust (`Forward::command`) serve the same path, header and JSON body, and both return the delegated `CommandRecord` or an error envelope, so one replayer drives both.

Lab-tagged Go and Rust `lab-harness` gateways append one JSON line per delegated `submit`, `status` or `list` to the file named by `APPLE_BRIDGE_LAB_FEED_LOG`: start and end wall times, the request body, the delegation's principal, grants, level, command id and request hash, and the outcome (`state`/`disposition`, `count` for list, or the error code). Requests that fail delegation verification are not logged. Production builds ignore the variable.

`shadow record --delegated-log <file>` (the file must not exist beforehand) merges those lines into the transcript at stop, by wall time, as `admission`/`response` rows with `ingress: feed`.

`shadow replay --feed-peer-home <home> --lab-binary bin/bridge-lab` re-drives each feed row with `bridge-lab shadow-feed`, which loads the synthetic replica from the peer home, signs a fresh delegation with the recorded principal and request hash, and posts to the clone's peer listener. The starting home must already enroll that replica (`bridge pair` then `bridge join --home <peer-home>`), the peer home is copied per gateway and the response outcome must equal the recorded one. Replay without `--feed-peer-home` fails closed when the recording has feed rows. `prove.py` pairs a synthetic replica before snapshotting the starting home, records a feed submit, status and list, and requires 8 of 8 exact Go-vs-Go replays plus zero Go-vs-Rust table differences (`--lab` sets the `bridge-lab` path).
## iPhone ingress

Lab Go (`Delivery.Handler`) and Rust (`ab-iphone` router) gateways accept `APPLE_BRIDGE_LAB_IPHONE_OBSERVER`, a Unix socket served by `shadow record --iphone-observer <socket>`. Before handling each `/v1/iphone/*` POST (including `/approval`, `/result` and `/pending`), the gateway sends the request's path, device ID, `at` and payload to the recorder and waits for its acknowledgment, then reports the response status and body afterwards. The recorder appends them as an `admission`/`response` pair with `ingress: iphone` and no signature, so they keep their order against the IPC frames the approval or result causes. An unreachable observer fails the request closed with 503.

`APPLE_BRIDGE_LAB_IPHONE_PLAINTEXT=1` lets both lab gateways serve `/v1/iphone/*` on the loopback API listener, which production requires TLS or the public host for. Replay sets it for the cloned gateways and never sets the observer.

`shadow replay --iphone-device <file>` re-drives those rows. The file holds a synthetic paired device ID and Ed25519 seed (`bridge-lab iphone-device --home <home> --output <file>` pairs one through the real pairing path before the starting snapshot). Each replayed request gets a fresh signature over the recorded payload. With a recorded clock, its `at` is the row's wall second and both gateways verify it, expire commands and sweep against the decision clock, so repeated replays are byte-identical. Without one it uses the current time. A recording from a real phone replays only against a starting home whose paired phone is replaced by the synthetic device.

`prove.py` pairs a synthetic device and an approval-level agent before the snapshot, then has the agent submit `iphone.ping`, and the device approve it, poll `/pending` and send a signed `observed` result. It requires the command to end `observed`, 8 of 8 exact Go-vs-Go replays and zero Go-vs-Rust table differences. The result is signed but not authenticated by Apple: it does not prove a real phone executed anything.
