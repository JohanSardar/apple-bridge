# Helper build modes

`packaging/dev/build-native.sh`, `build-helper.sh` and `build-truth.sh` default to the lab/dev mode. They define the Swift `LAB` compilation condition. Set `APPLE_BRIDGE_NATIVE_LAB=0` to build without it; only `0` and `1` are accepted. Both the SwiftPM and Command Line Tools paths use the same condition.

`packaging/build-app.sh` always selects `APPLE_BRIDGE_NATIVE_LAB=0`, including when its caller has selected lab mode. It produces the release Helper with the product commands and without the lab dispatch or its dedicated implementation types.

Product commands include `session`, `status`, `onboard` and the Messages `selftest` described in SPEC §23.13. The packaged app's install, uninstall and service-status entry points are unchanged. The following commands require a lab build:

- `reminders-store-blob-fixture`
- `reminders-store-proof`
- `reminders-store-flag-proof`
- `reminders-store-profile`
- `notes-probe`
- `functional`
- `probe`
- `wire-checks`
- `media-cleanup-checks`

A release Helper rejects these with `unknownCommand`, even when given valid lab arguments. Shared implementations used by product self-test, command safety and cleanup remain in release builds.

## Immutable signed apps

Give `build-helper.sh` and `build-truth.sh` a new destination for every World. Do not rebuild a running World's app or replace the shared installed dev app during verification. All modes use the existing signing script, certificate, bundle IDs and designated requirements.

The signed-app cache includes the normalized compilation condition in its source key. Default mode and explicit `1` share a key; `0` uses a distinct key. Compiler outputs and native-output pointers are separated by mode so a build cannot stage the opposite mode's binary. Lab consumers read `native-output-1`; release packaging reads `native-output-0`. Builds remove the obsolete `native-output` pointer. The existing cache signature, source and bundle-digest checks remain required.

`HELPER-RELEASE-COMMANDS` builds both signed modes, verifies their cache attestations and signatures, checks product command recognition, rejects lab commands in release mode, executes a safe wire fixture in lab mode, checks that dedicated lab types are absent from release symbols and builds the release package with caller LAB mode selected to verify the packaging override.

## Native checks at sim level

`make sim` builds the lab native targets before running scenarios. Simulation includes synthetic native media and usage decoder checks, so a Go build alone does not prepare the suite. Before running one of these scenarios directly through `bridge-lab`, run `make native` through `packaging/dev/heavy.sh`. Native output pointers belong to the current worktree and build mode.

Run isolated timing checks through the sensitive heavy lane, including scenarios whose names do not contain `perf`: `APPLE_BRIDGE_HEAVY_NO_SIM_LANE=1 packaging/dev/heavy.sh go run ./cmd/bridge-lab run <scenario.yaml> --level sim`. Preserve the host conditions with the result; a failed timing budget under contention requires a quiet rerun with the same threshold.

The recovery check keeps durable scan counts per source: Calendar revalidates its window on reconnect, while checkpoint-resumable sources must perform zero additional scans. Raw Truth comparisons omit the gateway-owned Notes `content_version`; gateway-to-replica comparisons retain the complete record, and `NOTE-LIST` checks that captured notes have valid content versions.
