# G-B2 lifecycle and local query integration

G-B3 owns `ab-commands` validation and ordered native-action encoding. G-B2 owns the durable lifecycle, native-record binding, upload leases, outcome correlation and local query/BlobRead routing. No second ledger or dispatcher is introduced. Error Code retains unknown native strings in Other(String), preserving Go rejection/error bytes; Code is Clone rather than Copy.

## Command seam

`ab_operations::command_contract::{validate,action,expiry}` adapts the existing operations Command into G-B3's types without losing RawValue argument bytes. Default `ActionResolver::validate` now calls the validated Go-compatible crate. Immediately before Prepare, lifecycle canonicalizes the resolved native context through `command_contract::prepared` and G-B3 native_action; it never sends JSON-map argument ordering. Resolvers must supply resolved root keys, staged files under messages.files and resolved resend_of under messages. Capture/PDF args may supply the staged file override. The immutable descriptor catalog is cached once per process; rebuilding it per query prevented a 100-call burst from fitting in the rate window.

Runtime installs an `ActionResolver` in `LocalGateway::new`:

- `borrow_uploads(principal,command)` returns BorrowedUploads with durable upload references and an optional pin/checkout lease held until ledger admission commits. When query_store is installed, lifecycle uses its production message_uploads/message_files adapters. Without that store, the optional resolver hook defaults to unavailable for nonempty files, matching Go.
- `serialization_key(principal,command)` chooses the object lock without checking the current version. The default derives Go object/list/Notes/files keys from validated arguments. GatewayStore resolves Calendar native series keys from visible records without checking versions. Version checks run under the acquired lock, allowing a queued expected future version to be validated after the preceding write commits.
- `resolve(principal,command)` returns `PreparedAction { action, expected_state, collection, serialization_key }`, with visible native-object/container/series bindings and version checks. It must perform no native effect.
- Lifecycle selects the serialization key, then calls `prepare(principal,command)` under that lock. The default forwards to resolve. Override prepare to return `(PreparedAction, Option<ActionLease>)`; retain staged-file cleanup in the lease until outcome processing completes.
- `collection(command)` supplies the collection for explicit Resolve, including messages.chat/reaction and container operations. It defaults to the legacy mapping.
- `conflict(principal,command,frame)` binds native conflict evidence to a committed capture and returns the current record.
- `evidence(command,request_hash,frame)` returns CaptureBound, Standalone, Artifact(HelperBlob) or Unproven. The default ports Go's completion checks for capture, playback, Mac, files, Focus, Maps and Shortcuts. CaptureBound retains capture receipt/observed-ID correlation and messages part strengthening. Artifact must hydrate successfully before observation is published. Failure remains indeterminate. The command/request/expiry bytes used for hashing remain unchanged by default-expiry calculation.

Recovery and old-era Resolve never dispatch. Matching stored IDs return their stored state before admission-era and new-expiry checks. Tests rotate only isolated ledger metadata and backdate the stored terminal expiry to exercise these rules; they are not an offline-restore qualification.

## Query seam

Use `LocalQueryStore { state: GatewayStore(shared_state), blobs: shared_blobs }`; the open constructor is only an isolated-fixture convenience. Assign it to `LocalGateway.query_store` before wrapping LocalGateway in Arc. Local queries implement Go's empty/Mac-extras argument checks, IDs/deadlines, mail-ID rewriting, document query/record blob authorization, bounded hydration and principal/scope-bound grants. Explicit JSON null results remain distinguishable from missing result fields. The query deadline covers document preparation and blob hydration.

`LocalQueryStore::document_input(principal,raw)` returns DocumentInput with public args and owned cleanup. Keep it alive until Helper work ends. Large inline data is staged as an owner-only file; the returned lease cleans it even when the await is cancelled. `query_blob(principal,id,digest,offset,length)` authorizes the stored grant before opening bytes; forwarding callers must bound length to 64 KiB. No Helper handle is returned to clients.

Set `LocalGateway.query_audit = Some(Arc::new(QueryActivity::open(home)?))` after State initialization to persist Go-shaped redacted agent query activity. A custom QueryAuditSink can additionally record operation, argument/result bytes and total blob bytes. Audit argument bytes reflect rewritten Helper inputs, matching Go. Cancelled futures notify the sink with completed=false; the activity adapter skips persistence then. Query input/errors, timeout-vs-Helper-deadline races, grant expiry, large-file cleanup, audit counters and rate admission have a process-driven probe.

## Cross-owner work still required

G-A: `State::save_query_blobs` currently ends with generic `crate::commit(tx)`; route it through the installed BlobProtection hook as Go commitProtected does. This must include expired query-owner deletion. G-B2 publishes a new query owner before saving its grant and releasing staging pins, preventing loss, but conservative orphan/expired protection may remain until State owns that transaction.

G-F: current source adapters and message staging are preserved. Install query audit and use the session-bound capture-blob interface in ab-applepeer.md. The library fixtures use a synthetic action resolver; source codec corpus success alone is not full gateway scenario qualification.
# Gateway blob grant parity

Blob reads return Unauthorized when the supplied record is absent, invisible, deleted, or does not reference the requested digest. Authorized records whose bytes are unavailable still return NotFound. This preserves the Go frontdoor's HTTP 401 behavior for invalid record grants without changing record reads or the wire format.
# Messages gateway command integration

LocalGateway persists owner-checked upload references on admission and materializes native file paths only for dispatch, with cleanup on completion or rejection. Service::upload_message backs authenticated POST /v1/blobs for local gateways with messages:write. Native capability_unproven errors retain their typed code. SourceState accepts Go null capability lists as empty without disconnecting the Apple peer. Messages action resolution is supplied by runtime; status resolution selects chat/reaction/message collections to match completion evidence.

Messages admission uses ab-commands::validate from PR #551 before storing a new command. Other source validators remain owned by the lifecycle integration worker. Upload references and recipient discovery still require gateway/Helper checks in addition to this pure validator.


FIX-CON-NOTE-REM: Source admission preserves the frozen `too_large` wire error from ab-commands validation and its HTTP 413 mapping.

Contacts, Notes and Reminders admission now uses ab-commands validation rather than generic catalog argument-shape validation. The latter rejects valid contact fields because the shared catalog input has an empty property map and a required ID/version even for create. Source validation preserves each operation's field/version rules before admission. Group membership resolve uses contacts.group, including add_member and remove_member.

Native dispatch rejections now use the same capture-bound conflict resolver as Prepare rejections. An external edit between Prepare and Dispatch returns the captured current record without redispatching the effect.

B3 coverage gap: ab-commands currently has no maps.open, mail command or pdf.annotate shapes/validators. The lifecycle preserves main's existing Maps validator and resolved raw action to avoid removing an already working operation. G-B3 must extend its codec corpus and API to cover these families before claiming every Prepare shape is byte-qualified; G-B2 does not author competing source contracts.
