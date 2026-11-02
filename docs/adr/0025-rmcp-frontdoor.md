# Use rmcp behind the front door

Status: Accepted subject to spike S4.

Use rmcp 3.8 behind ab-frontdoor, generated from handwritten operation descriptors without macros. Preserve local JSON responses and public SSE, with explicit Host and Origin allow-lists. Hand-roll only after a demonstrated S4 failure.

Compare initialize, tools/list, tools/call, errors, cancellation and legacy client behavior against go-sdk 1.8.0. Preserve auth scopes and timing metadata. Version selection remains subject to a buildable pinned lockfile. Frontdoor must not directly import state.
