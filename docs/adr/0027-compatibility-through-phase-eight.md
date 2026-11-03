# Freeze compatibility through Phase 8

Status: Accepted.

Until Phase 8 completes and Go is deleted, preserve schema_version=2, state.db and ledger.db formats, credentials, ipc/1, feed/1, relay/1, DER certificate fingerprints, iPhone HKDF salt apple-bridge-iphone/1 with device ID info, OAuth and passkey storage and the apple-bridge-owner user handle. Newly written state must reopen under the Go rollback binary.

A schema number alone is insufficient. Preserve raw signed bytes, JSON encodings, nanosecond timestamps, IDs and durable command evidence. No silent passkey re-enrollment. FINAL-PLAN Phase 7 permits later explicit format migrations; defer those until Phase 8 completion under this stricter compatibility freeze. Rollback swaps binaries on the same home and retains the newest ledger/guards; ledger restoration requires separate authorization.

## Phase 4 G-D1 addendum: passkey algorithm exception

Accepted by the orchestrator on 2026-11-02. The orchestrator inspected only the
COSE header of the sole live credential and confirmed ES256 (-7, P-256). No
secret values were read. Rust supports ES256 and EdDSA (-8, Ed25519) only.
RSA and other curves are unsupported; no RSA advisory waiver is retained.

The sole ceremony transcript exception is
`enroll_options.publicKey.pubKeyCredParams` in `contracts/golden/passkey.json`,
now exactly ES256 followed by EdDSA. Every other fixture byte, the owner handle,
credential envelopes, JavaScript, challenge and verification policy stay frozen.
`TestPasskeyGolden` names this exception explicitly and continues verifying the
original signed registration and login. The Go rollback implementation remains
able to read both supported key types.

Startup and doctor validate every stored passkey, including credentials for
other RP IDs. An unsupported algorithm or curve refuses startup with a re-enrol
hint; credentials are never removed or filtered to conceal incompatibility.
Re-enrollment remains a separate owner action using the Go binary before a
retry. Malformed credentials also fail closed.

The pinned webauthn-rs-core 0.6.2 and crypto-glue 0.2.2 sources are locally
patched under rust/vendor: RSA primitives and certificate verification removed,
Ed25519 strict verification added through ed25519-dalek. Upstream provenance,
licenses and patch scope are recorded there. These patches need revalidation
before upgrading either dependency.
