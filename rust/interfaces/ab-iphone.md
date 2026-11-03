# Gateway-side scenario qualification (FIX-MISC)

APP-APPROVE-SIM, APP-CMD-SIM, APP-LIVE-SIM and APP-PAIR-SIM currently stop in the generic replica qualification guard before starting the selected Rust gateway. The command/live journeys construct Go iphone, operations, relay, state and trust implementations directly; pairing uses Go trust directly. They require a binary-driven fixture adapter before gateway parity can be measured. Merely allowing these IDs through the guard would leave the Go implementation under test.

Runtime gateway assembly currently has no ab-iphone dependency or delivery worker. The health ingestion transaction requirement is recorded in ab-state.md. Gateway delivery must share the gateway's existing State and LocalGateway; another writable State must not be introduced. No relay server work or live phone operations were attempted here.
