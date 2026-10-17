# Apple Bridge — Astra response, round 1

2026-10-16. Response to [Claude’s round 1](claude-r1.md), including its appended user decision: Mac-accessible data only, with no iPhone app or iPhone-only APIs. That supersedes both designs’ phone components. The accepted twenty critique findings remain requirements. Research below is documentary, not testing on Johan’s Mac.

## D1. Process split and placement — agree

**Keep the gateway on the Air by default.** Swift owns Apple access; the shared gateway/replica executable owns replication, persistence, REST, and MCP. Ubuntu remains an ordinary replica. Ship the Mac components together with authenticated local IPC and coordinated upgrades.

The portable gateway could run elsewhere eventually, but that is not a v1 deployment option. Remote adapters would require durable source-side buffering, reconnect/replay, and command fencing; an IPC boundary alone does not provide those guarantees. Neither the protocol nor client installations should hard-code Ubuntu’s identity.

## D2. Shared service language — agree; I concede Go

Your distribution argument outweighs my TypeScript preference. Agent interoperability comes from the wire contract, not implementation language. The [official Go MCP SDK](https://github.com/modelcontextprotocol/go-sdk) removes the main ecosystem objection. My earlier critique already recognized Go’s suitability; the independent design should converge accordingly.

Use Go with `modernc.org/sqlite` provisionally. Its [documentation](https://pkg.go.dev/modernc.org/sqlite) confirms a cgo-free implementation and support for the relevant desktop platforms. Promise tested, self-contained executables per supported OS/architecture, rather than universally static binaries.

I do not accept the unmeasured memory ratio as evidence. Benchmark the actual daemon on the 8 GB Air, including backfill, FTS indexing, reconciliation, and blob transfer. Validate FTS5, WAL recovery, and packaging in shipped builds. These are implementation gates, not reasons to retain Node.

## D3. Screen Time — compromise; accept the Mac investigation

I omitted a useful path. Add an **experimental, read-only Mac collector**. Apple confirms that [Share Across Devices synchronizes Screen Time data](https://support.apple.com/en-lamr/guide/mac-help/wrp2jwrzghnm/mac). Under Johan’s updated scope, iPhone-origin usage qualifies only when it already reaches the Mac.

However, synchronized reports do not establish a complete, readable event store. An existing [Screen Time backup implementation](https://github.com/alexjmiller5/screentime-backup) reports that macOS 26.3 denies access to ScreenTimeAgent/DeviceActivity stores even with Full Disk Access, while knowledgeC and Biome remain readable. Its companion [dashboard](https://github.com/alexjmiller5/screentime-dashboard) distinguishes recorded aggregates from focus-derived sessions with inferred or capped intervals. This supports feasibility, not equivalence to Apple’s totals; the observations are the author’s, not independently reproduced here.

Probe Johan’s exact OS build with SIP enabled. Demonstrate device attribution, app identity, retention, missed intervals, and agreement with controlled usage and the Mac’s Screen Time reports before advertising coverage. Distinguish local-Mac activity from iPhone-origin records. Preserve provenance and mark reconstructed durations as estimates.

Remove the proposed phone Shortcuts, DeviceActivity, FamilyControls, and shielding paths under the new scope. EU export eligibility is consequently no longer an architectural dependency. We agree on investigating the Mac path; its actual coverage remains an empirical question, not a tiebreaker dispute.

## D4. Offline writes — agree; I concede the default

Fail fast when the required gateway or adapter is known unavailable. My original queue-by-default wording was too broad, especially for messages. Optional delayed execution requires explicit opt-in, `expires_at`, and operation-specific maximum TTLs; it can follow v1.

Keep the durable command ledger for online execution. Recheck expiry and authorization immediately before dispatch, including after wake. A no-queue command that loses adapter readiness before dispatch must fail, not wait for its return.

One qualification: a lost response after possible acceptance is **indeterminate**, not proof of rejection. Recover status using the original command ID; never invite a fresh-ID resend. Fail-fast admission cannot eliminate ambiguous execution outcomes.

## D5. Health bootstrap — agree with the updated user scope

To your original clarifying question: I had no architectural objection to an explicitly partial export bootstrap. **Johan’s subsequent Mac-only decision now rules out requiring it.** Remove the companion app and any dependency on new iPhone Health Auto Export or Shortcuts setup.

Health remains eligible only if an actual source is already accessible on the Mac. Inventory that source before promising support. An existing export or third-party Mac cache may be mirrored with its provenance, coverage, and freshness; its presence does not establish complete Health history or a write-through path to Apple Health.

If no source exists, advertise Health as unavailable. If only a readable export exists, advertise that limited capability and reject writes. Do not invent a Mac Health store merely because data exists in iCloud, or turn the excluded phone app into a hidden prerequisite.

## D6. Notes — agree

After the core. Ship only demonstrated read/write capabilities. Unsupported rich-content edits must return an explicit error; neither AppleScript success nor a plaintext round trip proves preservation of attachments, formatting, or collaboration state.

## D7. Contacts write scope — disagree with deferral

A difference omitted from your list: your draft §5 postpones `contacts.create/update`, whereas my coverage includes native Contacts writes. I favor **basic contact create/update/delete in v1 alongside Contacts replication**. Apple exposes these through [CNSaveRequest](https://developer.apple.com/documentation/contacts/cnsaverequest); this is a supported write path, unlike speculative Screen Time extraction or advanced Messages operations.

Johan explicitly wants a general read/write system. Once we implement Contacts identity, permissions, and change capture, shipping only lookup leaves a preventable gap in that contract. Use the same command ledger, conflict handling, and observed-result verification as other writers; respect account writability. Advanced group management can follow. A read-only milestone is acceptable during development, but I disagree with making it the v1 Contacts endpoint.

## Unresolved

- **D7 — Contacts release scope:** Astra wants basic contact CRUD alongside Contacts replication in v1; Claude’s draft defers contact create/update to later and omits delete from v1.
