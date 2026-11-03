# G-F gateway assembly

The runtime constructs one shared Gateway State for ApplePeer, LocalGateway, StateReads<Gateway>, feed and owner control. It installs blob protection before capture or listeners. Service and Frontdoor accept the Local target, LocalQueryStore shares the same State and Blobs, and Web is outside bearer middleware with its own authenticated owner approval adapter. The gateway owns shutdown, Helper supervision, feed dialing, pruning, blob hydration and status/GC workers.

G-B3: runtime currently installs a Reminder-only ActionResolver in ab-runtime/src/actions.rs to exercise lifecycle assembly. Replace this adapter with the production resolver from ab-commands when available; do not duplicate the shared ledger, peer or dispatcher. The current adapter supplies visible IDs, native list keys, expected state/version, serialization keys and capture-bound native conflict errors. Other source actions return Unsupported. This limited adapter does not qualify source-specific admission or completion validation.

The peer listener delegates authenticated direct /v1/commands and feed CommandRequest to one runtime UploadHandler wrapper. Upload requests remain in G-C's Uploads; command requests use LocalGateway. TrustIdentity rechecks delegated replica activity/grant ceilings and LocalGateway recognizes delegated agents for approval levels. Challenge-based reauthorization of previously admitted delegated approvals remains pending G-B2/G-C integration.

Online backup uses the private run/control.sock listener and the existing shared State under the gateway's singleton lock. Offline backup and restore retain the shared lock. No second writable State is opened by online backup.

Phone delivery and atomic health capture, remote ingress/public transport assembly, source production validation and remaining harness conversions are not qualified by this slice. See ab-state.md for the phone transaction request. APP-*-SIM remains unqualified. Air builds are currently blocked by free disk below the Phase 4 budget.
