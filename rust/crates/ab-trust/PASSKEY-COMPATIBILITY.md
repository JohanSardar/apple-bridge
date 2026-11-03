The orchestrator approved ES256/P-256 and EdDSA/Ed25519 only, after confirming
that the sole live credential has algorithm -7. See ADR 0027's G-D1 addendum.
RSA is removed from the dependency tree and no advisory exception remains.
Local patches of the pinned core and crypto facade are described in
rust/vendor/README.md. Startup and doctor reject unsupported stored keys with
an explicit re-enrol hint; they never drop credentials.
