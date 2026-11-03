# Gateway-side scenario qualification (FIX-MISC)

APP-APPROVE-SIM, APP-CMD-SIM, APP-LIVE-SIM and APP-PAIR-SIM currently stop in the generic replica qualification guard before starting the selected Rust gateway. The command/live journeys construct Go iphone, operations, relay, state and trust implementations directly; pairing uses Go trust directly. They require a binary-driven fixture adapter before gateway parity can be measured. Merely allowing these IDs through the guard would leave the Go implementation under test.

Runtime gateway assembly currently has no ab-iphone dependency or delivery worker. The health ingestion transaction requirement is recorded in ab-state.md. Gateway delivery must share the gateway's existing State and LocalGateway; another writable State must not be introduced. No relay server work or live phone operations were attempted here.

## G-F binary assembly proof

The runtime installs Delivery<PhoneGateway> against its shared Gateway State and common LocalGateway. The independent Python binary fixture exercises real pairing CLI, authenticated Tailscale forwarding, Ed25519/X25519/HKDF/AES-GCM packets, common ledger results, phone approvals, location dedupe and revocation. Both Go and Rust pass the 12 common checks; Rust additionally preserves a running Tailscale adapter across relay selection changes. This is assembly evidence, not APP scenario qualification.

The Go lab binary accepts an isolated LocalAPI socket and loopback public probe target behind the lab build tag. The probe still verifies TLS and performs the production OAuth/MCP/Web/60-second SSE checks; ordinary Go builds retain their existing LocalAPI and public DNS behavior.

Capture upload is not present in ab-iphone yet. Runtime needs the signed /v1/iphone/capture endpoint to share command authorization and serialization with Delivery, stage into the existing blob Store, and persist caller-bound query blob grants in the existing State. Health still requires the atomic State seam described in ab-state.md.
