# Helper native/1 adapter

The default Helper session still connects directly with ipc/1. Set
`AB_SESSION_PROXY=1` to launch the bundled `Contents/MacOS/ab-session` child.
`AB_SESSION_BINARY` overrides that executable for a lab build. The parent connects
and checks the gateway socket, then passes fd 198 (gateway), fd 199 (raw IPC),
and fd 200 (separate native channel) without disclaiming responsibility. Only the
child receives `AB_NATIVE_FD=200`. The existing authentication and IPC handlers run
unchanged across the raw proxy. Child termination closes the session; reconnect
starts a fresh child. The direct connection never creates these socketpairs.

The native channel stays idle until the child sends Hello. `NativeServer` wraps
`NativeAdapter`; `AppleSession` supplies the framework adapter. Frames are capped
at 256 KiB. Full reads use pinned, bounded-lifetime snapshots and snapshot-bound
cursors. Intermediate and incomplete pages never claim completeness. Fact scans
use separate source instances and storage under `native-facts`, preserving the
IPC capture source state. Current source adapters accept full native scope with
empty args; filtered requests fail unsupported. Calendar scope is the existing
CalendarTime window. Contacts history accepts `history_token` and returns the next
opaque token and changed identifiers, or expired when a full scan is necessary.

Effects remain disabled on the proxy channel while Swift owns IPC command
authority. `AppleSession.runNative(descriptor:)` is the exclusive entry point for
a later Rust routing cutover; it cannot run beside `run()`. The current Rust raw
proxy does not perform that cutover. This prevents activating a second dispatch
guard during Phase 6 step 1. Native permits bind the operation, canonical argument
hash, owner fence, deadline, command, ticket and one-shot nonce. Existing source
prepare, validation, dispatch and read-back remain authoritative. An invoked
adapter failure yields indeterminate evidence and permanently consumes the nonce
for that connection. Rust must never replay an ambiguous effect.

Native command args contain the existing command envelope, including action,
expected_state, origin, principal and the original IPC request_hash. The action op
is bound to the native effect operation. Framework query args retain their
existing shape. Document queries accept `file_ref` instead of arbitrary `file`
paths. Large effect results use session-owned file refs. Release takes
`args.file_ref` and replies Released. File helpers verify ownership, mode, regular
file type, hardlink count, size and digest. Media derivatives reuse the existing
engine and export its derivative bytes as file refs.

Build checks in this checkout, never in the live Helper home:

```
python3 packaging/dev/generate-source-registrations.py native
~/bin/ab-local env APPLE_BRIDGE_NATIVE_TARGET=NativeChecks python3 packaging/dev/build-native.py native
~/bin/ab-local native/.build/clt-release/NativeChecks lab/fixtures/native-1/facts.json
```

The fake-adapter executable checks permit rejection/replay, exact fixture facts,
paging/completeness, real socketpair exchanges, frame bounds and file confinement.
It does not prove live framework behavior or authorize a production cutover.
