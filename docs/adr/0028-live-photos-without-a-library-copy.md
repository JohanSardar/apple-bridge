# Live PhotoKit queries without a library copy

Status: Proposed, native qualification and independent security review pending.

Photos follows `location.current` rather than the replicated source model. Swift owns PhotoKit authorization, fetches, resources and native change/readback. Rust owns source scopes, argument validation, agent approvals, identity binding, single dispatch and gateway/replica routing. Frontdoor only translates the shared operation descriptors.

No photo or album collection is added to the domain or state schema. Query results and original bytes do not enter state, snapshots, durable query blobs or replication. Readiness contains only permission state. Large originals use asynchronous private temporary transfers and bounded reads under the same authenticated query channel. Temporary files unlink before any photo bytes arrive; only open descriptors retain data, which the kernel removes on process exit or a crash.

Search uses creation-date ordering and bounded PhotoKit fetches, including a single lookahead candidate. Native qualification found that PhotoKit rejects sorting or range predicates on `localIdentifier`. Equal-date ties instead use the public `SELF` exclusion predicate with already examined native objects. Undated assets sort last; tie ordering is unspecified. Coordinate filtering advances across rejected candidates too, so an empty page can still continue. No exact full-library count or growing offset-prefix fetch is required.

Opaque cursors reference memory-only native checkpoints bound to unchanged filters. Checkpoints expire after ten minutes or a Helper restart. At most 128 checkpoints and 131,072 retained object references exist; older checkpoints are evicted first and return `expired`. A single tie group exceeding that capacity returns `unavailable`, never an incomplete end-of-library result. Permission loss clears checkpoints. No checkpoint enters Bridge state, snapshots or replication.

Organization writes also use the native query envelope. The existing durable approval state machine retains a canonical request digest instead of private arguments. Every agent must obtain owner approval, then reissue the same request. Owner device requests dispatch directly but consume their IDs. Completion stores only success and the request hash. Failed or interrupted dispatch is indeterminate and cannot be replayed.

Hash-only durable records are supplemented by a bounded memory-only preview in the owner's browser. Approval fails closed when that preview has expired or was lost on restart; an identical pending request restores it. The paired iPhone cannot approve Photos changes until its preview protocol supports this transient model. Independent security review remains required before qualification.

Public PhotoKit only. No Photos database reads or private people-name search. No asset deletion or permanent deletion. Regular-album membership removal and album deletion must not delete assets.
