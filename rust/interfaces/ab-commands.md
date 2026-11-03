# ab-commands — command validation and Helper argument parity

Owner: command-validation-shapes implementation worker (separate from G-B2 / PR #541).

Taking: Go contracts source validators, command ID / wait / expiry helpers, and ordered gojson NativeAction argument serialization, with a Go-generated parity corpus. G-B2 keeps admission, authorization, state lookup, uploads, version conflicts, tickets, dispatch, outcome, resolve and per-part evidence aggregation. No lifecycle files are edited here.

Interface: `validate(&Command, now: jiff::Timestamp) -> Result<Validated, Error>`; Command contains `id`, `op`, raw JSON `args`, `expected_version`, `wait_ms`. Validated exposes target `id` and `list_id`. `native_action(&Command, &ActionContext) -> Result<Vec<u8>, Error>` returns ordered HTML-escaped Go JSON bytes; context supplies resolved native keys, staged message files and resolved resend key. `expiry(op, now, requested)` must run after stored-ID lookup. `origin` derives trusted principal provenance; never accept it from submitted input.

Helper policy remains authoritative for recipient discovery, route proofs, lab gates, parts and variant capability: Go contracts reject unknown `parts` and `variant` fields. Do not broaden that surface. Owner self origin applies only to local owner tokens and messages.send; the unchanged Helper verifies own recipients again at dispatch. Calendar count-to-end normalization occurs in the unchanged Helper, not the Go gateway; preserve count in Prepare.

The implementation is available in `rust/crates/ab-commands`; the Go oracle and regeneration commands are in `tools/command-vectors`.

## G-B2 integration contract

- Add `ab-commands` as a dependency of the lifecycle crate; `Command.args` is a boxed RawValue to preserve lexical input (especially contacts).
- `validate` receives the wall clock and returns target IDs only. It performs no reads or effects. Run before admission; the validated target IDs do not replace visibility checks or native-key lookup.
- After authorization derive `origin(op, kind, principal_id, remote, agent_level)` from the authenticated principal. Discard caller-supplied origin. The Helper remains responsible for verified self recipients and dispatch revalidation.
- After dedupe, check admission era and call `expiry`; zero Go expiry means `None`. Preserve request expiry for hashing rather than inserting the default expiry into the submitted command.
- Before Prepare construct ActionContext from resolved native/list/folder keys, staged message file names/paths/digests and the resolved resend native key. Resend must first pass Go's from-me/error/chat checks in the lifecycle.
- `native_action` returns the exact ordered NativeAction JSON. `prepare` adds IPCFrame ordering, owner fence and normalized GoTime. Pass these bytes through the peer exchange; decoding/re-encoding with a JSON map loses the required ordering and raw argument escapes.
- Capture/print staging is lifecycle work: substitute the resolved local file and clear blob/blob_id before encoding. iPhone commands use delivery rather than Helper Prepare.
- Notes current-state HTML compatibility is exposed as `note_html_compatible`. Folder writability, content-version conflicts, locks and recently-deleted checks remain lifecycle work. `clock_estimate` returns offset and uncertainty in nanoseconds.

No ab-operations or ab-applepeer lifecycle source is edited by this worker. Frozen IPC golden checks plus the new Go oracle compare validators and Prepare encoding; they do not claim runtime wiring or native-effects parity.

## Verification evidence

The Go-generated corpus has 2,980 validator vectors (accept/reject, exact error code and safe message), including 540 successful NativeAction and complete Prepare byte comparisons. A separate 65-vector corpus covers expiry, clock estimates and Notes HTML structure. Regeneration uses the product Go contracts and typed structs; no personal data or live gateway was used.

`make rust-ci` passes on Ubuntu, including lint, workspace tests, formatting, strict Clippy, dependency policy and frozen golden corpora. After rebasing onto gateway assembly PR #546, all nine `core.commands` sim scenarios passed with `gateway=rust,replica=go` on Ubuntu. The harness root was set to an isolated disk-backed `APPLE_BRIDGE_LAB_ROOT` because the host's shared `/tmp` quota was exhausted. These scenarios verify the existing gateway lifecycle, while the new source-validator crate is proven independently by the Go corpus; lifecycle integration must explicitly call the interface above.
