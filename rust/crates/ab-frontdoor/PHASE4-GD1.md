> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# G-D1 OAuth, consent and passkeys

Continues PR #539 at 425973a5, preserving the original commits and attribution.
Main through G-F PR #542 is merged. No live home, service, credential or device was accessed.

Implemented: Go-shaped OAuth clients, PKCE, agent grants, hashed credentials,
refresh rotation and persisted reuse revocation; device sessions and enrollment;
server-rendered consent, passkey, approval and command pages; pending connector
control; bounded public metadata fetches; frozen JavaScript.

The orchestrator approved ES256/P-256 and EdDSA/Ed25519 only after confirming
the sole live passkey is ES256. ADR 0027 records the single frozen-transcript
exception: enroll_options.publicKey.pubKeyCredParams. All other golden fields
and signed ceremony bytes remain identical. HTTP options match the frozen Go
shape and bytes after replacing random challenge/ceremony IDs. Rust omits the
core's default explicit attestation=none and empty allowCredentials to match Go.

Patched pinned core/crypto-glue sources under rust/vendor remove RSA primitives
and add strict Ed25519 verification. cargo-deny has no advisory exceptions.
Authority::open and doctor validate every stored credential, across RP IDs,
and reject unsupported algorithms/curves with a re-enrol hint. No credentials
are silently discarded. Malformed envelopes fail closed.

G-F owns the gateway assembly. The complete Web/Approvals/listener/TLS/control
channel contract is in rust/interfaces/ab-frontdoor.md. This PR proves Rust auth/pages with
a Go backend; it does not qualify an assembled Rust gateway or live cutover.

Reproduce on Ubuntu in an isolated checkout (Go and Cargo on PATH):

```sh
export APPLE_BRIDGE_SECRET_STORE=file
make rust-ci
cargo build --manifest-path rust/Cargo.toml -p ab-trust --example oauth_interop --example passkey_algorithms -p ab-frontdoor --example consent_fixture
go build -o /tmp/ab-gd1-oauth-go ./rust/crates/ab-trust/interop
python3 rust/crates/ab-trust/interop/run.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/oauth_interop" /tmp/ab-gd1-oauth-go
python3 rust/crates/ab-frontdoor/interop/golden.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/consent_fixture"
"${CARGO_TARGET_DIR:-rust/target}/debug/examples/passkey_algorithms" "$(mktemp -d)"
cargo build --manifest-path rust/Cargo.toml -p ab-runtime --bin bridge
python3 rust/crates/ab-trust/interop/startup.py "${CARGO_TARGET_DIR:-rust/target}/debug/bridge"
go build -tags lab -o bin/bridge ./cmd/bridge
go build -o bin/bridge-lab ./cmd/bridge-lab
python3 rust/crates/ab-trust/interop/consent.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/consent_fixture" /tmp/ab-gd1-consent.json
```

Proof scope: five OAuth golden exchanges, full registration/login HTTP option
bytes, 49 OAuth/credential interoperability assertions including Go ES256 golden
login, unchanged 38-assertion CONSENT-CONNECTOR against Rust auth with Go backend,
fresh passkey interoperability both ways, synthetic Ed25519 registration/login,
signature tampering rejection, and unsupported stored key refusal without writes.
The additional Go test logs the golden algorithm exception explicitly. No
existing behavioral scenarios were weakened; the consent fixture uses a Go
overlay and does not count Go backend/MCP execution as Rust evidence.

The actual Rust start/doctor probe uses an unsupported key under another RP ID and verifies the diagnostic, nonzero exit, unchanged credential bytes and unchanged home entries. The CLI preserves Trust errors so the hint reaches the operator.
