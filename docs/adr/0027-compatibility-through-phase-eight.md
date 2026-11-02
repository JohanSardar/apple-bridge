# Freeze compatibility through Phase 8

Status: Accepted.

Until Phase 8 completes and Go is deleted, preserve schema_version=2, state.db and ledger.db formats, credentials, ipc/1, feed/1, relay/1, DER certificate fingerprints, iPhone HKDF salt apple-bridge-iphone/1 with device ID info, OAuth and passkey storage and the apple-bridge-owner user handle. Newly written state must reopen under the Go rollback binary.

A schema number alone is insufficient. Preserve raw signed bytes, JSON encodings, nanosecond timestamps, IDs and durable command evidence. No silent passkey re-enrollment. FINAL-PLAN Phase 7 permits later explicit format migrations; defer those until Phase 8 completion under this stricter compatibility freeze. Rollback swaps binaries on the same home and retains the newest ledger/guards; ledger restoration requires separate authorization.
