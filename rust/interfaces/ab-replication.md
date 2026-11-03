# Phase 3 CRUD investigation, 2026-11-02

See ab-state.md for native evidence classification, regression details, and remaining qualification gaps. No ab-replication product change was justified by a failing reproduction. Existing mixed Go gateway/Rust replica sims CORE-REPLICA-CAPTURE-ATOMIC (including Calendar and byte-limited batches), CORE-RESUME, and CORE-SNAPSHOT-UNDER-LOAD passed before changes. The missing crash boundary was in ab-state lab instrumentation, not feed batching or resume.
# G-F command listener assembly

The runtime supplies one UploadHandler wrapper: requests with method route through LocalGateway after VerifyDelegation; upload control/chunks and close notifications delegate to G-C Uploads. Server's TLS peer listener now handles POST /v1/commands through that same callback. Direct replies preserve error envelopes and lookup-only headers; feed replies retain CommandResponse correlation.

The runtime also starts Server::dial concurrently with listeners. Fresh challenge-based delegated approval reauthorization is not yet connected; previously admitted delegated approvals must fail closed until that interface is available.
