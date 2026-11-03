# Gateway blob grant parity

Blob reads return Unauthorized when the supplied record is absent, invisible, deleted, or does not reference the requested digest. Authorized records whose bytes are unavailable still return NotFound. This preserves the Go frontdoor's HTTP 401 behavior for invalid record grants without changing record reads or the wire format.
# Messages gateway command integration

LocalGateway persists owner-checked upload references on admission and materializes native file paths only for dispatch, with cleanup on completion or rejection. Service::upload_message backs authenticated POST /v1/blobs for local gateways with messages:write. Native capability_unproven errors retain their typed code. SourceState accepts Go null capability lists as empty without disconnecting the Apple peer. Messages action resolution is supplied by runtime; status resolution selects chat/reaction/message collections to match completion evidence.

Messages admission uses ab-commands::validate from PR #551 before storing a new command. Other source validators remain owned by the lifecycle integration worker. Upload references and recipient discovery still require gateway/Helper checks in addition to this pure validator.


FIX-CON-NOTE-REM: Source admission preserves the frozen `too_large` wire error from ab-commands validation and its HTTP 413 mapping.

Contacts, Notes and Reminders admission now uses ab-commands validation rather than generic catalog argument-shape validation. The latter rejects valid contact fields because the shared catalog input has an empty property map and a required ID/version even for create. Source validation preserves each operation's field/version rules before admission. Group membership resolve uses contacts.group, including add_member and remove_member.

Native dispatch rejections now use the same capture-bound conflict resolver as Prepare rejections. An external edit between Prepare and Dispatch returns the captured current record without redispatching the effect.
