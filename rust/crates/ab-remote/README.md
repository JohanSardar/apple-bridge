# Remote ingress

The installed Tailscale app is the only remote route. `bridge tailscale` records the selection with `state::select` in `remote/ingresses.json`; the runtime reconciles that selection and publishes `remote/status.json`.

`Tailscale::tailnet(home)` uses the Unix LocalAPI, requires the HTTPS capability, chooses the first free port among 8443, 9443 and 4443, and retries ETag CAS five times. It persists its lease before changing Serve. Removal verifies every owned entry and preserves unrelated handlers.

A random capability path protects the loopback listener. Requests need the exact Host and forwarded headers and a tailnet source address; requests Tailscale forwards from the public internet are refused. Ordinary HTTP connections close after one request; WebSocket upgrades stay duplex.
