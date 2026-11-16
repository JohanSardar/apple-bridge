# Drop embedded tsnet and use the installed Tailscale app

Status: Accepted.

The Rust product has no embedded tsnet or Go sidecar. The installed Tailscale app's LocalAPI provides tailnet access: `bridge tailscale` adds an owned Tailscale Serve mapping and preserves every unrelated handler. TLS terminates in tailscaled on the Mac, and only devices on the owner's tailnet can reach the gateway.
