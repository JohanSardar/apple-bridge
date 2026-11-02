# Drop embedded tsnet and use relay plus the installed Tailscale app

Status: Accepted migration direction; operational qualification pending one-week soak. Supersedes ADR 0016 only after that gate.

The Rust product has no embedded tsnet or Go sidecar. Relay is the public default; the installed Tailscale app LocalAPI provides tailnet and optional Funnel access. Existing Go embedded/private code stays available for rollback during Phase 0.

Zero-install private clients change requirements. Preserve relay enrollment and existing origins where possible; explicitly qualify changed origins/passkeys. Phase 0 switches the Go live gateway, checks the access paths, then soaks for one week. Failed qualification rolls back with remote on --via private; no credential/state deletion. R0 containment precedes Rust relay work; relay/1 is not qualified for sensitive replicas.

Phase 0 finding: Go selects a single remote backend. Rust ab-remote must allow relay and the installed Tailscale adapter to run concurrently. The retained relay hostname was not serving under the prior private backend either; an EOF after the system switch is not a regression.
