# Writes fail fast when the gateway is unreachable

v1 has no offline command queue: a replica that cannot reach the gateway returns `unavailable` immediately, and a lost response is recovered by looking up the original command ID. Delayed execution would send texts hours late; it may come later only as explicit opt-in with `expires_at` and per-operation maximum TTL.
