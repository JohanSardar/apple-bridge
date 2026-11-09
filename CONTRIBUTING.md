# Contributing

Start with [AGENTS.md](AGENTS.md), [principles](PRINCIPLES.md), the [glossary](GLOSSARY.md) and the relevant [architecture decisions](docs/adr/). Discuss substantial behavior changes in an issue before implementing them. Use synthetic examples in public reports.

## Build from source

Source builds are a development path, not a public installer. Use Rust with edition 2024 support, Python 3, make, and on macOS the matching Swift 6 toolchain and Apple SDK. Signing and native permissions require separate local setup under packaging/dev. Main does not need Node or pnpm.

```sh
make build
make lint
make features
make rust-ci
```

`make build` builds the Rust workspace and, on macOS, the native Helper. Linux builds do not validate Apple integrations. `make rust-ci` includes tests, formatting, Clippy, cargo-deny, lab independence, native graph and golden checks; install cargo-deny separately. Use `make rust-golden` for descriptor/OpenAPI compatibility. The feature checker writes design/features.html; avoid committing incidental generated changes.

```sh
make sim SCENARIOS=lab/scenarios/core/CONN-CHATGPT.yaml
make real SCENARIOS=lab/scenarios/core/CONN-CHATGPT.yaml
make replica
```

Run only relevant scenarios. Real and replica checks require configured hosts and explicit authorization for native data access. Do not run them against somebody's live home. See [lab interfaces](rust/interfaces/) and [release packaging](packaging/release/README.md).

On an existing development host, `~/bin/ab-local` may serialize checks. It is a private host utility, not included or required by this repository. `rust/check.sh` uses it if present; otherwise it invokes Cargo directly. Use a private CARGO_TARGET_DIR for remote builds and remove your scratch checkout afterwards. Never build on a low-memory host running the live gateway.

## iPhone integration

The gateway owns pairing, `/v1/iphone/*`, command admission, the ledger, signed record ingestion and relay/APNs delivery. The separate [iPhone repository](https://github.com/JohanSardar/apple-bridge-iphone) owns device execution, permissions, keys and receipts. Mac builds need neither its checkout nor the iOS SDK.

Only scenarios that compile phone models or drive the phone harness need it:

```sh
git clone https://github.com/JohanSardar/apple-bridge-iphone.git ../apple-bridge-iphone
APPLE_BRIDGE_IPHONE_ROOT=/path/to/apple-bridge-iphone make sim SCENARIOS=lab/scenarios/iphone/APP-SHELL-SIM.yaml
```

Gateway pairing and delivery checks run here without that checkout. The [feature graph](features.yaml) links app-owned modules to it.

## Branches and pull requests

The full SolidJS web app is preserved on the `web-app` branch. Main serves consent, passkey enrollment and agent approvals; agent management uses `bridge agent`.

Keep PRs focused. Explain the user-visible result and provide exact checks and known limits. Public-interface doc comments and required safety justifications are allowed; other code comments remain prohibited. Security or data-loss code changes require independent review; other changes do not. Report vulnerabilities [privately](SECURITY.md).

Historical committed evidence has owner names and local home prefixes redacted for publication. It is not fresh verification; verdicts and measured values are retained. Bundle identifiers and wire fixtures are preserved for compatibility.
