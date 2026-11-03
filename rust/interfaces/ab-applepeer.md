
# G-B: ApplePeer and local gateway integration

`ab_operations::peer::ApplePeer::read_blob(handle, offset, length)` returns a cancellation-aware future of `(Vec<u8>, done)`. `ab_applepeer::Commands` implements it with a distinct per-session, per-handle response registry. The request length caps at 128 KiB; duplicate reads of one handle return Conflict; unavailable/offloaded/other missing reasons map to Unavailable/Unsupported/NotFound. Disconnect clears waiters; timeout shuts down the session. Blob replies bypass capture commits, allowing capture-side blob hydration without deadlocking its response.

G-F can construct `LocalQueryStore { state: GatewayStore(shared_state), blobs: shared_blobs }` and assign `LocalGateway.query_store = Some(Arc::new(store))` before wrapping LocalGateway in Arc. The `open` constructor is for isolated fixtures; production must share G-C's blob Store. Mail body queries resolve a visible bridge ID before sending the native ID. Query results hydrate up to sixteen 64-MiB blobs, verify digest/length, stage with pins, save principal/scope-bound query grants, publish protection and strip Helper handles. The query deadline covers hydration. Staging pin cleanup also runs when the query future is cancelled during a blocking stage. Query admission caps at 100 calls per principal per second.

G-A follow-up required: `save_query_blobs` currently commits through generic `crate::commit`, bypassing the installed gateway BlobProtection hook. Route gateway query grants and expired query-owner deletion through the same protection transaction as capture. G-B conservatively publishes each successful query owner before releasing staging pins; expired-owner cleanup still needs the State hook. G-F must not treat query grant expiry/GC as qualified until this lands.

Document input grants/staging, query audit/activity hooks and source completion checks are implemented by the lifecycle follow-up. Native encoding/validation comes from G-B3; main supplies production runtime record binding and upload staging. See ab-operations.md for the runtime audit/session hooks. The isolated probes use synthetic identity/action adapters; full gateway qualification still requires the assembled scenario matrix, shadow replay and orchestrator-owned real smoke.

## Session-bound blob routing

`ab_operations::peer::PeerSession` is an opaque-in-use owner-fence token. ApplePeer::session returns the active token; Commands::read_blob_from(token,handle,offset,length) rejects a replaced session before sending bytes. QueryResult and Outcome frames carry their authenticated session's fence in memory for hydration; it is never part of returned query/evidence JSON. Local query/capture hydration retains that response token across all chunks rather than selecting the current Helper after reconnect.

`Backend::commit_from(PeerSession,Capture)` is invoked by Server for captured frames. It defaults to commit for existing adapters. G-F should override it and retain the supplied token in queued capture-blob jobs; HelperBlobs should use read_blob_from. Calling unbound read_blob for old queued handles does not establish ownership of those handles after reconnect. The process-driven Go/Rust probe retains a reader, replaces the Helper and requires unavailable when reading through the old owner.

## G-F LocalGateway matrix follow-up

With session-bound hydration, owner CLI and LabClock installed, CORE-CMD-CLOCK-JUMP and capture-observer checks pass. WEB-AGENT-LEVELS still accepts a changed body under an existing pending command ID instead of returning Conflict; compare request hashes before returning pending status. Replica delegated approval currently fails closed through TrustIdentity::approval_principal. CORE-QUERY-ROUNDTRIP's native REST path passes; MCP test queries need the existing lab-queries feature enabled in the matrix artifact. Runtime query audit file wiring follows separately.
