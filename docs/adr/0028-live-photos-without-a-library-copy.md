# Live PhotoKit queries without a library copy

Status: Proposed, native qualification and independent security review pending.

Transfer read lengths are upper bounds, not promised chunk sizes. Reads return at most 64 KiB so worst-case slash escaping in base64 plus the query envelope stays below the existing 256 KiB native frame limit. Clients advance by decoded bytes until `eof`. The frame limit and accepted request lengths remain unchanged.

Photos follows `location.current` rather than the replicated source model. Swift owns PhotoKit authorization, fetches, resources and native change/readback. Rust owns source scopes, argument validation, agent approvals, identity binding, single dispatch and gateway/replica routing. Frontdoor only translates the shared operation descriptors.

No photo or album collection is added to the domain or state schema. Query results and original bytes do not enter state, snapshots, durable query blobs or replication. Readiness contains only permission state. Large originals use asynchronous private temporary transfers and bounded reads under the same authenticated query channel. Temporary files unlink before any photo bytes arrive; only open descriptors retain data, which the kernel removes on process exit or a crash.

Search uses creation-date ordering and bounded PhotoKit fetches, including a single lookahead candidate. Native qualification found that PhotoKit rejects sorting or range predicates on `localIdentifier`. Equal-date ties instead use the public `SELF` exclusion predicate with already examined native objects. Undated assets sort last; tie ordering is unspecified. Coordinate filtering advances across rejected candidates too, so an empty page can still continue. No exact full-library count or growing offset-prefix fetch is required.

Opaque cursors reference memory-only native checkpoints bound to unchanged filters. Checkpoints expire after ten minutes or a Helper restart. At most 128 checkpoints and 131,072 retained object references exist; older checkpoints are evicted first and return `expired`. A single tie group exceeding that capacity returns `unavailable`, never an incomplete end-of-library result. Permission loss clears checkpoints. No checkpoint enters Bridge state, snapshots or replication.

Organization writes also use the native query envelope. The existing durable approval state machine retains a canonical request digest instead of private arguments. An agent the owner set to ask first must obtain owner approval, then reissue the same request. Owner device and `full` agent requests dispatch directly but consume their IDs ([ADR 0030](0030-connected-apps-act-by-default.md)). Completion stores only success and the request hash. Failed or interrupted dispatch is indeterminate and cannot be replayed.

Hash-only durable records are supplemented by a bounded memory-only preview in the owner's browser. Approval fails closed when that preview has expired or was lost on restart; an identical pending request restores it. The paired iPhone cannot approve Photos changes until its preview protocol supports this transient model. Independent security review remains required before qualification.

Public PhotoKit only. No Photos database reads or private people-name search. No asset deletion or permanent deletion. Regular-album membership removal and album deletion must not delete assets.

## Revision: showing and sending the photo itself

Agents showed the person a numbered contact-sheet cell, a 172 KiB thumbnail, as if it were the photo. `photos.fetch` gains `rendition: "display"`: the Helper renders the current version at full size through `PHImageManager`, applies orientation and encodes a metadata-free JPEG, at most 8,000 px on the longest side, re-encoded or scaled only as far as `max_bytes` requires. It uses the existing temporary transfer, so large images never exceed the native frame. MCP `photos_show` returns it as image content.

Two deliberate exceptions to "no photo bytes on disk or in blobs", both made only by an explicit agent call: `photos_show` with `save` writes the original under the bridge home (`blobs/photo-files`, `0700`/`0600`, removed after 24 hours by the next save), and `messages_send` with `photos` stages each photo file, as Photos shows it, as an ordinary message upload with that upload's expiry. Neither enters state, snapshots or replication.

People, scene and text search, and titles, captions and keywords, exist publicly only in the Photos app's scripting dictionary, behind an Automation permission the owner has not granted. They stay out of scope.

## Revision: place names

Agents asked which places the owner visited had to guess from thumbnails: about 70 calls and 10 minutes for one question. PhotoKit exposes coordinates but not the place names Photos shows, so the Helper turns coordinates into names with MapKit's `MKReverseGeocodingRequest`, which needs no Location Services permission. Each search asset with a location gains `place`, and `photos.places` groups a date range's photos by place with counts, first and last dates and one representative asset, plus country and US state rollups.

Lookups go to Apple's geocoding service with one coordinate per 0.05° cell, the first photo seen there, never the photo. Answers stay in Helper memory only (at most 50,000 cells, cleared on restart) and never enter Bridge state, snapshots, replication or disk, so "no library metadata is stored" still holds. Apple rate-limits geocoding: lookups run one at a time under a per-call budget, a refusal pauses them for a minute, and `photos.places` reports `complete: false` instead of guessing. Screenshots are skipped by default because they rarely carry the owner's location.
