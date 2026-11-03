# G-F gateway assembly

MSG gateway parity: runtime Actions maps Messages visible IDs to native keys, carries native Messages args and expected state, validates resend targets, and chooses chat/reaction completion collections. SIM Messages journeys use the existing isolated process-backed world for the Rust gateway and Go replica; real-level qualification remains gated.

The runtime constructs one shared Gateway State for ApplePeer, LocalGateway, StateReads<Gateway>, feed and owner control. It installs blob protection before capture or listeners. Service and Frontdoor accept the Local target, LocalQueryStore shares the same State and Blobs, and Web is outside bearer middleware with its own authenticated owner approval adapter. The gateway owns shutdown, Helper supervision, feed dialing, pruning, blob hydration and status/GC workers.

The runtime ActionResolver in ab-runtime/src/actions.rs supports Reminders, Contacts and Notes using ab-commands validation and Helper argument serialization. It shares the ledger, peer and dispatcher. The resolver supplies visible IDs, native object/container keys, expected state/version, serialization keys and capture-bound native conflict errors. Other source actions return Unsupported. Source-specific completion validation beyond capture binding remains unqualified.

The peer listener delegates authenticated direct /v1/commands and feed CommandRequest to one runtime UploadHandler wrapper. Upload requests remain in G-C's Uploads; command requests use LocalGateway. TrustIdentity rechecks delegated replica activity/grant ceilings and LocalGateway recognizes delegated agents for approval levels. Challenge-based reauthorization of previously admitted delegated approvals remains pending G-B2/G-C integration.

Online backup uses the private run/control.sock listener and the existing shared State under the gateway's singleton lock. Offline backup and restore retain the shared lock. No second writable State is opened by online backup.

Phone delivery now shares the gateway State and LocalGateway, with source validation delegated to ab-commands. The runtime mounts signed pairing/status/delivery, persists signed result evidence, routes phone approvals through the command lock with hash verification, and commits location captures under the shared State lock. Pending phone commands survive startup without Helper recovery. Atomic health ingestion remains unavailable pending the State transaction seam recorded in ab-state.md; delegated phone command reauthorization remains fail closed. APP-*-SIM remains unqualified until its in-process Go fixtures are converted.

Public HTTP transport context comes from a private authenticated loopback connection used only after ab-remote verifies TLS or the Tailscale capability path and headers. Caller headers cannot mark local API HTTP secure. Runtime reconciles relay and system selections independently, retains healthy adapters when another selection changes, and derives accepted public hosts from active adapter readiness. Frontdoor, Web and phone routes share the Service/Authority/Approvals adapters. The Gateway::new and Gateway::authenticated constructors replace external struct literals; examples use the appropriate constructor.

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

## CORE gateway clock diagnostics

Doctor reports clock_skew=n/a for gateway homes, matching Go. Four-timestamp gateway clock samples and their actionable warnings apply to replica homes only.

## FIX-MISC sim actions

The portable FILE-COMMANDS / FILE-CRUD-REAL / FILE-OFFLOAD-REAL, USE-*, AUD-SIM, LOC-SIM and MAP-SIM journeys use world.New/newEnvironment and the selected gateway process. The generic replica qualification guard rejected these gateway runs before startup; only these audited sim IDs are eligible for gateway qualification. All scenario assertions are retained.

Files actions serialize on `files`, keep raw FileArgs, and require both operation-specific readback and capture-bound record evidence. Maps open and playback are transient actions: completion requires the prepared request hash plus Maps frontmost / playback finished callback evidence, rather than a nonexistent captured record. Native actions stay behind the same LocalGateway ledger, Prepare/Dispatch guard and authenticated ApplePeer.

MAC-*, PDF-*, VIS-OCR, IWORK-REAL, SC-RUN-REAL and RELAY-* still have semantic Go helper or direct product imports in their harness paths. They are blocked before a Rust product failure can be measured; enabling those guards alone cannot qualify Rust. The relay server port is outside FIX-MISC ownership.

The current catalog's maps.open/speech.say/capture.play_sound input schemas contain ReminderArgs and require expected_version. Keep the frozen exported catalog intact; Service::invoke and LocalGateway::submit use the operations' dedicated argument validators for these three operations instead. Files and playback validation call the newly merged ab-commands parity implementation; Maps validation remains local because ab-commands currently reports Maps operations unsupported.

FIX-CON-NOTE-REM: the runtime resolver now admits Contacts and Notes alongside Reminders using ab-commands validation and ordered native_action bytes. It resolves visible object/group/member/folder keys, translates group expected members back to native references, applies Notes content-version and folder/HTML restrictions, and binds native conflicts to the correct source capture. The sim qualification guard permits these source families' existing isolated binary simulator paths; real and replica qualification remains unchanged. Notes blob helpers use the contracts BlobProgress wire type rather than importing the Go blob implementation. No scenario assertions change.

## G-F gateway lab hooks

With lab-harness only, the runtime exposes the owner-only run/blobs-lab.sock control fixture through its existing shared State and blob Store. Stage/release, frozen snapshot leases and the synthetic relay blob capture call product APIs. The synthetic capture is committed before releasing its staging pin. Capture crash gates park after the State commit and before the IPC acknowledgment; the command capture FIFO gate parks before command receipt resolution. Ordinary builds include neither the listener nor the gates.

Unchanged CORE-BLOB-NATIVE-STAGE, CORE-BLOB-STAGE-DURING-TRANSFER, CORE-BLOB-STAGE-SAME-DIGEST and CORE-KILL-GATEWAY-CAPTURE pass against Go and Rust. Command-reference GC control awaits the State seam requested in ab-state.md; it must not be emulated in the fixture.

## G-F session and owner ceremony wiring

Capture hydration carries the authenticated PeerSession from Backend::commit_from through each queued blob request and calls Commands::read_blob_from. Replaced owners cannot satisfy old handles; sessionless captures containing handles fail before State commit. Runtime installs the provided QueryActivity sink and lab-only LabClock on the single LocalGateway.

The CLI supports web enroll --host / --public using Authority::create_web_enroll, and agent create/list render the public Go-shaped summary (scopes, timestamps, no credential hashes). An explicitly empty agent level selects approve, matching Go. Enrollment terminal QR rendering remains outside this slice.

## G-F query audit assembly

Runtime's QueryAuditSink appends the Go-shaped operation/argument_bytes/result_bytes/blob_bytes row to owner-only commands-audit.jsonl and forwards completed agent queries to the provided QueryActivity sink. It never logs query arguments or results. The lab-harness feature includes the existing frontdoor test-query descriptors so the default matrix reaches the same test.echo/test.sleep/test.blob paths over REST and MCP; ordinary builds retain the frozen catalog. CORE-QUERY-ROUNDTRIP, CORE-QUERY-DEADLINE, CORE-QUERY-BLOB and CORE-QUERY-SCOPE pass unchanged against the selected Rust gateway. Full Linux matrix: 116 pass / 35 fail / 50 blocked (201), with CAL-WINDOW-SLIDE passing in this run after previous intermittence.

## G-F APP-LOC-SIM binary qualification

The location scenario retains the Swift fake adapter proof and sends its five location records twice through the selected gateway's signed HTTP ingress. Readback uses records.query; each of the five location/geofence commands is admitted through REST, decrypted with the independent phone wire decoder, completed through signed HTTP and read back through commands.status. Invalid geofence validation uses HTTP, and coordinate-bearing receipts remain rejected. All behavioral assertions pass against Go first and then Rust on the Air, serially through ab-local; no real location adapter or device permission request runs.

## G-F owner approval metadata

The runtime Approvals adapter renders the same redacted argument metadata as Go: argument_bytes counts the original raw JSON bytes and argument_hash is their SHA-256 fingerprint. The authenticated owner_arguments field retains the full pretty JSON. WEB-AGENT-LEVELS owner_receives_full_text now passes; its pending hash conflict, delegated authorization and activity failures remain explicit. Linux remains 116 pass / 35 fail / 50 blocked. The latest separate full Darwin matrix is 14 pass / 25 fail / 23 blocked, including two 50-second timeouts counted as failures; all 62 default cases ran serially. Its binary source is 05de2f4b and does not include this metadata-only change.

## G-F lab catalog CLI

lab-harness builds expose lab-operation-catalog before opening Authority or the requested home, returning catalog scope/schema entries and the three provided test-query descriptors. MSG-CAPABILITY-MATRIX passes against the selected Rust binary. A direct Go oracle without lab registration matches all 147 ordinary product descriptors exactly. Full lab metadata is not equal: Go alone registers labdummy.item in the three read collection enums, and the provided test query descriptors clone helper.version input rather than their operation-specific argument schemas. Those remaining owner seams are retained as failed full-lab comparison evidence, not hidden by modifying exported schemas in runtime.

## G-F current ownership: Mac command adapter

G-F is wiring non-capture mac.* commands through the existing ab-commands validator/NativeAction codec and the existing LocalGateway completion evidence checks. This is runtime-only source work; G-E retains the portable SIM eligibility audit. Capture/document staging and missing source validators remain separate product seams. No real Mac action or permission request is authorized.

MAC-COMMANDS is already an isolated World.New SIM journey with selected REST/MCP/Helper binaries and no Go implementation calls. G-F temporarily adds its narrow SIM eligibility for the Go/control and runtime resolver regression; G-E can replace that entry with the broader audited portable SIM policy. Real-level and replica eligibility do not change.

Non-capture Mac commands now use ab-commands::native_action with authenticated origin and the same fallback serialization key as Go. Existing LocalGateway validation, approval policy and hash/readback evidence checks remain authoritative. Go passes the five isolated Mac/iWork SIM controls; Rust passes MAC-ACTIONS, MAC-CAPTURE, MAC-EXTRAS-SIM and IWORK-REAL at sim level. MAC-COMMANDS now passes execution/readback and fails only the retained clipboard redaction/durable-file assertions requested from the lifecycle owner. The full Linux run is 149 pass / 32 fail / 20 blocked; CAL-WINDOW-SLIDE failed after passing in the preceding run. No real action or permission request ran.
