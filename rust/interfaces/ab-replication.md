# Phase 3 CRUD investigation, 2026-11-02

See ab-state.md for native evidence classification, regression details, and remaining qualification gaps. No ab-replication product change was justified by a failing reproduction. Existing mixed Go gateway/Rust replica sims CORE-REPLICA-CAPTURE-ATOMIC (including Calendar and byte-limited batches), CORE-RESUME, and CORE-SNAPSHOT-UNDER-LOAD passed before changes. The missing crash boundary was in ab-state lab instrumentation, not feed batching or resume.
