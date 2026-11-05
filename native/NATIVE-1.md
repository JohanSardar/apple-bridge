# Helper native/1 adapter

The Helper connects and checks the gateway socket, then starts its bundled `Contents/MacOS/ab-session` child without disclaiming
responsibility, passing fd 198 (gateway), fd 199 (session link) and fd 200 (facts, `AB_NATIVE_FACTS_FD=200`). `AB_SESSION_BINARY`
overrides that executable in a lab build only; a release build verifies the bundled binary and the gateway peer (see docs/helper-build-modes.md). ab-session authenticates and owns ipc/1; Swift speaks only native/1. Child termination closes the session,
and reconnecting starts a fresh child.

Commands reach Swift only as `Primitive` frames. `Ready` and `FrameworkStatus` carry raw status per source; ab-session
turns it into coverage, capabilities and state. `NativeServer` wraps `NativeAdapter` on the facts stream; `AppleSession` supplies the framework adapter. Frames are capped
at 256 KiB. Full reads use pinned, bounded-lifetime snapshots and snapshot-bound cursors. Intermediate and incomplete pages never
claim completeness. Current source adapters accept full native scope with
empty args; filtered requests fail unsupported. Calendar scope is the existing CalendarTime window. Facts enumerate only what the framework returns now:
no baseline, no absent rows, no `ObservationSink`. `Revalidate` answers a framework fact (`present`, `missing`, `outside_scope`) per
identity and `Changed` is pushed on the same stream; frames are handled concurrently by id. Rust owns the diff and every deletion decision.

Build checks in this checkout, never in the live Helper home:

```
python3 packaging/dev/generate-source-registrations.py native
~/bin/ab-local env APPLE_BRIDGE_NATIVE_TARGET=NativeChecks python3 packaging/dev/build-native.py native
~/bin/ab-local native/.build/clt-release/NativeChecks lab/fixtures/native-1/facts.json
```

The fake-adapter executable checks exact fixture facts, paging and completeness, real socketpair exchanges and frame bounds.
It does not prove live framework behaviour.
