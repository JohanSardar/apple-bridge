# G-D1 implementation and remaining merge gate

This change starts from iPhone foundation `4fcaadc2`. No live home, gateway,
passkey, browser or device was accessed. The JavaScript asset is copied verbatim
from `internal/web/assets/webauthn.js`.

Implemented modules:

- `ab-trust::oauth`: Go-shaped dynamic clients, authorization codes, PKCE S256,
  agent principals, one-hour access, absolute 90-day refresh rotation, revocation
  on reuse, enrollment ceilings, credential-lock transactions and hashed storage.
- `ab-trust::devices`: Go device session and enrollment token envelopes,
  trusted/public idle and absolute lifetimes.
- `ab-trust::passkeys`: the 18-byte owner handle through webauthn-rs-core 0.6.2,
  Go credential import, registration envelope export, current counter checks and
  backup flags. Existing credential fields survive login updates.
- `ab-frontdoor::web`: consent, enrollment, login, approval and command pages,
  metadata, registration, authorization and token routes, pending connectors,
  security headers, origin/host checks, strict cookies and request limits.
- `web_connector`: bounded expiring pending approval state, five matching-code
  attempts and single-use polling. Runtime can call `pending_connectors` and
  `decide_connector` from its owner-only control channel.
- `web_metadata`: public-IP-only HTTPS metadata fetches, pinned DNS results,
  bounded bodies, redirect validation and HTTP cache lifetime handling.

Measured on Ubuntu:

- `make rust-ci`: passed, including lint, workspace tests, formatting, Clippy,
  all cargo-deny checks and existing Go golden checks.
- OAuth golden HTTP transcript: five exchanges, zero byte mismatches.
- Credential interoperability: 49 assertions, zero failures, all four
  Go/Rust issuance/exchange combinations, both readers, refresh expiry,
  hashed secrets, rotation and persisted family revocation.
- Existing `runConsent` scenario: 38 assertions, zero failures, using a Rust
  frontdoor fixture with a Go gateway backend. The virtual authenticator also
  proves fresh Go registration → Rust login and Rust registration → Go login.
  The original scenario and its assertions are unchanged; a Go overlay adds
  the fixture adapter. Backend command execution and MCP are explicitly Go.

## Do not merge as an exact drop-in yet

The pinned core does not implement every algorithm advertised by Go. In
particular, its COSE importer accepts RSA only for RS256 with a 256-byte modulus
and three-byte exponent, and its public-key verifier does not implement EdDSA.
Go's frozen registration options also advertise RS384, RS512 and RSA-PSS.
Consequently full passkey option/transcript and legacy algorithm parity is not
established. The current evidence proves ES256 compatibility. This needs an
upstream correction or a separately reviewed compatibility implementation;
silently narrowing Go's stored-credential contract would violate the brief.

The RUSTSEC-2023-0071 exception is documented in Trust's
`PASSKEY-COMPATIBILITY.md`; cargo-deny passes with only that advisory waived.
This does not solve the algorithm coverage gap.

The gateway runtime remains unwired. Construct `Web` with the actual bound
address, the public host and an `Approvals` adapter, and route these requests
separately from the API bearer middleware. Set `SecureTransport` only from an
actual TLS listener; forwarded headers never establish secure transport. The
`Approvals` adapter supplies redacted/full argument strings and delegates
current owner authorization and decisions to the command authority.

Reproduce (Ubuntu, isolated checkout):

```sh
export APPLE_BRIDGE_SECRET_STORE=file
cargo build --manifest-path rust/Cargo.toml -p ab-trust --example oauth_interop -p ab-frontdoor --example consent_fixture
go build -o /tmp/ab-gd1-oauth-go ./rust/crates/ab-trust/interop
python3 rust/crates/ab-trust/interop/run.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/oauth_interop" /tmp/ab-gd1-oauth-go
python3 rust/crates/ab-frontdoor/interop/golden.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/consent_fixture"
go build -tags lab -o bin/bridge ./cmd/bridge
go build -o bin/bridge-lab ./cmd/bridge-lab
python3 rust/crates/ab-trust/interop/consent.py "${CARGO_TARGET_DIR:-rust/target}/debug/examples/consent_fixture" /tmp/ab-gd1-consent.json
```

The golden HTTP fixture binds only Ubuntu loopback port 7780. Startup failure
never stops another listener. The consent fixture uses a second loopback address
on the Go gateway's random port, keeping the OAuth resource and RP origin
identical across the two frontdoors. Only command/backend API routes are proxied;
Rust owns auth, OAuth, cookies and rendering. No browser automation is involved.
