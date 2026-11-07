# Photos, live through PhotoKit

Photos is optional. `bridge permissions guide` offers its own Photos step; skipping or declining it does not disable another source. Status checks, gateway startup and read operations never request access. A missing grant fails closed. Limited-library access is reported explicitly.

`bridge read photos --limit 20 --json` reads the current library, not a synced collection. REST `/v1/op/photos.search`, MCP `photos_search`, local agents and remote replicas use the same live query path. The Mac must be awake and connected. Photos does not participate in `records.query`, full-text search, background import, blob replication or library counts in `bridge stats`.

## Operations

Every request uses `{"id":"qry_<random-id>","args":{...},"deadline_ms":5000}`. Use random request IDs, never photo details. Results are under `result_json`.

`bridge call` remains read-only by default. Use `bridge call --list --write` to include the six Photos organization writes. Transfers and writes require `--token-env PHOTO_TOKEN`, naming an environment variable containing an existing credential with the necessary scopes. Keep the same credential for transfer start, reads and release; the CLI neither creates nor revokes it. Never put the credential itself in command arguments.

For example, `bridge call photos.favorite --write --token-env PHOTO_TOKEN --args '{"id":"qry_<random-id>","args":{"asset_id":"<asset-id>","favorite":true}}'`. Writes require explicit `--write` and a stable query ID. These flags do not bypass scopes or agent approvals. No other mutating operation is enabled by `--write`.

| Operation | Arguments and effect |
|---|---|
| `photos.search` | Half-open RFC3339 `start`/`end`, `type`, `album_id`, `favorite`, explicit `include_hidden`, optional `near: {lat,lon}` and `radius_m`; `cursor` and `limit` (1–200). |
| `photos.get` | `asset_id`; current dimensions, duration, dates, favorite/hidden flags, coordinates if present, burst identity, and the complete public resource inventory. |
| `photos.albums` | `smart` selects smart versus user albums. Page with `offset`/`limit`. Each album reports editing capabilities. |
| `photos.fetch` | Start with `asset_id`, optional `resource_index`, `resource_key`, `network_access` (default false), `max_bytes` (default 8 GiB, at most 64 GiB). Without an index, select the first original photo/video/audio resource by public type. Poll/read with only `transfer_id`, `offset`, `length`. |
| `photos.release` | `transfer_id`; cancel and delete the temporary resource. |
| `photos.favorite` | `asset_id`, explicit boolean `favorite`; set and read back. |
| `photos.album_create` | `title`; returns the new album ID after readback. |
| `photos.album_rename` | `album_id`, `title`; rename and read back. |
| `photos.album_add` / `photos.album_remove` | `album_id`, unique `asset_ids` (1–200); change memberships and read back. Removal does not delete assets. |
| `photos.album_delete` | Delete an editable regular album, not its assets; verify album absence. |

Search bounds the PhotoKit fetch itself: `limit + 1` candidates without a coordinate filter, or 1,001 with one. At most 1,000 candidates are examined per page; the extra candidate detects whether another page exists without counting the full library. `examined_count` counts only this page's examined candidates, before coordinate filtering. Follow `next_cursor` with unchanged filters even if the current page is empty. A null cursor ends enumeration. The cursor carries the last examined creation date and identifier, preserving equal-date ties and including undated assets last. Results are live, sorted newest first then by identifier, not snapshot-consistent. Concurrent edits can change membership, but later pages do not refetch and skip the earlier prefix. Search no longer accepts numeric offsets or reports an exact full-library count.

Place search uses explicit coordinates, not Photos' inferred place names. Use the existing geocoding operation to turn an address into coordinates. Types include image, video, audio, Live Photo, screenshot, panorama, HDR, depth, slow motion and time lapse. Unknown media subtype bits remain visible in details.

## Originals, Live Photos and iCloud

Inspect `resources` from `photos.get` first. PhotoKit exposes original photos/videos/audio, alternate RAW/JPEG representations, paired Live Photo video, full-size edited representations and adjustment resources. Choose the resource index from that response. Pass its `resource_key` to reject a changed resource inventory or asset modification time at transfer start. This is a metadata precondition, not a byte digest or a library snapshot. Fetch the still and paired video separately for a Live Photo; nothing silently converts or flattens it.

The start call returns a transfer ID immediately. Poll until `state` is `ready`; each read returns SHA-256, total size, offset, base64 `bytes`, and `eof`. Read chunks of at most 512 KiB. Verify the assembled SHA-256 and call `photos.release`, including on failure. `network_access:true` allows PhotoKit to obtain iCloud-only resources; offline or unavailable content fails rather than appearing empty.

An expired or invalidated transfer returns `expired`; start a new transfer. A stale `resource_key` returns `conflict`; refresh `photos.get` before choosing the resource again. These are distinct from an unavailable resource.

Four transfers may exist per Helper. Poll or read within 120 seconds to retain one, including during an iCloud download; every transfer has a monotonic 24-hour hard lifetime. A 15-second sweep closes abandoned transfers. Native resource callbacks write directly to private temporary files and incrementally hash bytes. Files open in read/write mode with permissions verified as `0600`, then unlink before receiving any photo bytes. Only open descriptors retain the data, so release, failure, expiry or process exit removes it, including a crash. iCloud progress and received byte counts are reported while downloading.

Transfer IDs are bound to the authenticated principal by Rust; caller-supplied ownership is rejected. An identical start request ID reuses its active transfer rather than starting another download. Files are not imported into Bridge blobs or the database, and no named media cache exists. Disk-space checks require a 2 GiB reserve. Large resources require sufficient scratch disk and a sufficient `max_bytes`; expiry or capacity errors are explicit.

Every transfer read rechecks that PhotoKit still exposes the asset, including under limited-library selection. Losing access invalidates and closes the transfer rather than serving its retained bytes. This native revocation behavior still requires a macOS limited-selection test.

## Scopes, approvals and retries

Reads and downloads require `photos:read`. Album/favorite changes require `photos:write`; a read-level agent cannot write even if misconfigured with that scope.

Photos writes use the transient query envelope so the command ledger never stores photo IDs, titles, coordinates, returned metadata or image bytes. Every agent, including `full` and delegated agents, first receives `{id,state:"awaiting_approval"}`. The owner approves through the existing approval UI, then the agent retries the identical operation, ID and arguments once. Approval itself dispatches nothing.

All six writes require a caller-supplied query ID, including direct owner requests. Neither the local gateway nor a forwarding replica may invent one for a mutation. This prevents an ID-less transport retry from becoming a fresh write.

The durable approval record contains only the operation and a SHA-256 digest of canonical arguments. A ten-minute, memory-only preview, bounded to 128 entries and 16 MiB total, lets the owner's browser display the exact private arguments without saving them. After a gateway restart, approval fails closed until the agent reissues the identical pending request to restore its preview. Photos approvals currently use the owner's browser, not the paired iPhone. Approvals expire after ten minutes and bind identity, operation and arguments. Owner device and scoped owner-token calls dispatch directly but also consume their request ID once.

A lost response, timeout, interrupted process or failed readback may leave an effect ambiguous. The request remains consumed and is never redispatched automatically. Inspect the live library before making a new explicit request. `commands.status` can inspect the retained hash-only record.

## Public API inventory and decisions

Based on Apple's public [Photos framework](https://developer.apple.com/documentation/photos), [fetch options](https://developer.apple.com/documentation/photos/phfetchoptions), [resource types](https://developer.apple.com/documentation/photos/phassetresourcetype), [album changes](https://developer.apple.com/documentation/photos/phassetcollectionchangerequest) and [library](https://developer.apple.com/documentation/photos/phphotolibrary) documentation:

- Asset/collection fetches, metadata, regular/smart albums, all asset resources and iCloud resource loading are used directly.
- Favorite changes and regular album create/rename/membership/delete are included. PhotoKit edit capabilities are checked before changes. Smart/shared/synchronized albums are not mutated.
- No private Photos database access, private people-name index, face recognition, inferred semantic search or undocumented cloud state. Public PhotoKit does not expose the Photos app's full people/search index.
- No asset deletion, permanent deletion, imports, original-content editing, hiding assets, or date/location rewrites. These change the library's underlying content or visibility, rather than the requested organizational surface. Existing assets remain untouched by album deletion.
- No persistent change observer/token, metadata mirror, thumbnail cache, prefetch/caching manager or background upload extension. They conflict with an on-demand, no-copy source or add no necessary capability to these operations.
- Legacy persisted `photos.*` collections remain on disk but are never decoded, served or exported by this live source. Photos permission/readiness and hash-only approval records remain valid. Malformed known readiness records still fail closed.
- Folder editing, custom album ordering, content-editing extensions, rendered previews and cross-device cloud-ID conversion are not exposed in this implementation. This is a bounded first implementation, not full PhotoKit API coverage.

## Qualification

`features.yaml` and `lab/evidence/photos-938.json` are the evidence of record. Linux tests qualify authorization, hash-only approval binding, single dispatch, forwarding and contracts, not PhotoKit or macOS permission behavior. A signed clean-VM run and an isolated read-only real-library run are required before this source can be considered qualified.
