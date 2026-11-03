# Phase 3 CRUD investigation, 2026-11-02

See ab-state.md for native evidence classification, regression details, and remaining qualification gaps. No ab-replication product change was justified by a failing reproduction. Existing mixed Go gateway/Rust replica sims CORE-REPLICA-CAPTURE-ATOMIC (including Calendar and byte-limited batches), CORE-RESUME, and CORE-SNAPSHOT-UNDER-LOAD passed before changes. The missing crash boundary was in ab-state lab instrumentation, not feed batching or resume.
# G-F command listener assembly

The runtime supplies one UploadHandler wrapper: requests with method route through LocalGateway after VerifyDelegation; upload control/chunks and close notifications delegate to G-C Uploads. Server's TLS peer listener now handles POST /v1/commands through that same callback. Direct replies preserve error envelopes and lookup-only headers; feed replies retain CommandResponse correlation.

The runtime also starts Server::dial concurrently with listeners. Fresh challenge-based delegated approval reauthorization is not yet connected; previously admitted delegated approvals must fail closed until that interface is available.

## CORE gateway clock parity

The feed server negotiates `clock_protocol=ntp/1` only when offered in Hello. Both normal and gateway-dialed sessions handle ClockPing/ClockAck through the shared authenticated upstream path, including while waiting for frame acknowledgments. ClockPong preserves the probe ID and t0, stamps gateway receive/send times, and leaves t3 zero. ClockAck rejects zero timestamps, reversed intervals, and negative network round trips. No wire format changes.

## CORE blob unavailability parity

Authenticated BlobWant requests receive the manifest state (`offloaded`, `failed`, `missing`, or `pending`) when bytes cannot be opened. Requests without a visible record grant receive `unauthorized` without consulting the manifest state. This lets Go replicas preserve offloaded metadata instead of retrying it as generic unavailable data. Wire formats remain unchanged.
