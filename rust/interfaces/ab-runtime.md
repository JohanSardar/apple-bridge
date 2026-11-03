# G-F gateway assembly

The runtime constructs one shared Gateway State for ApplePeer, LocalGateway, StateReads<Gateway>, feed and owner control. It installs blob protection before capture or listeners. Service and Frontdoor accept the Local target, LocalQueryStore shares the same State and Blobs, and Web is outside bearer middleware with its own authenticated owner approval adapter. The gateway owns shutdown, Helper supervision, feed dialing, pruning, blob hydration and status/GC workers.

G-B3: runtime currently installs a Reminder-only ActionResolver in ab-runtime/src/actions.rs to exercise lifecycle assembly. Replace this adapter with the production resolver from ab-commands when available; do not duplicate the shared ledger, peer or dispatcher. The current adapter supplies visible IDs, native list keys, expected state/version, serialization keys and capture-bound native conflict errors. Other source actions return Unsupported. This limited adapter does not qualify source-specific admission or completion validation.

The peer listener delegates authenticated direct /v1/commands and feed CommandRequest to one runtime UploadHandler wrapper. Upload requests remain in G-C's Uploads; command requests use LocalGateway. TrustIdentity rechecks delegated replica activity/grant ceilings and LocalGateway recognizes delegated agents for approval levels. Challenge-based reauthorization of previously admitted delegated approvals remains pending G-B2/G-C integration.

Online backup uses the private run/control.sock listener and the existing shared State under the gateway's singleton lock. Offline backup and restore retain the shared lock. No second writable State is opened by online backup.

Phone delivery and atomic health capture, remote ingress/public transport assembly, source production validation and remaining harness conversions are not qualified by this slice. See ab-state.md for the phone transaction request. APP-*-SIM remains unqualified. Air builds are currently blocked by free disk below the Phase 4 budget.

## G-F CAL gateway sim integration

Calendar commands resolve through the shared runtime Actions adapter into a calendar-specific module. Prepare carries calendar_args plus native_key, expected_list_key and list_key resolved from visible state. Mutations preserve expected-version conflict records and serialize recurring occurrences by calendar/series. Native calendar conflicts bind the calendar capture checkpoint before returning current state. span=all returns unsupported before admission on REST/MCP and direct LocalGateway submissions. No capture deletion semantics change.

The default CAL sim journeys use the selected gateway binary and Go replica over public REST/MCP, ipc/1 and feed/1. Their explicit Rust-gateway qualification is limited to the 21 default matrix IDs; scenario definitions and assertions remain unchanged. This does not enable real-level journeys or qualify Rust replica combinations.

Verification: original product after unblocking the harness passed 10/21 CAL sims and failed 11 on command-dependent paths. Final Go and Rust gateway matrices each pass 21/21. CAL-SPLIT-FRAME-CAP had latency failures in the initial Go run and two intermediate Rust runs; all its functional assertions passed after the resolver fix, and the final runs passed latency too. Per-scenario results, intermediate failures, binary digest and the unchanged exactly-once evidence limitations are retained in `crates/ab-runtime/evidence/phase4/gateway-calendar.json`. `make rust-ci` passes on the rebased patch. No native calendar investigation was run.
## CORE unavailable capture blobs

Capture hydration preserves explicit offloaded/missing reference states instead of calling Store::begin and replacing them with pending. The authenticated feed reports those states to replicas; no blob transfer is scheduled until a later capture supplies an available reference.

## CORE scope control

The gateway CLI exposes `scope <replica-id> --scope <grants>` through Authority::change_scope. Trust persists the new grants and increments the revision; existing feed sessions close on revision mismatch, and replicas reset/backfill against the new revision. No trust or feed contract changes.

## CORE direct command envelope

Direct submit requests may omit the query field, as in Go. Forward defaults the unused CommandQuery and continues through delegation verification before command admission. Forged grants still produce Unauthorized without a native effect.
