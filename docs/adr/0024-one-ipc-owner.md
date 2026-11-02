# Keep exactly one external ipc/1 owner

Status: Accepted.

ab-session first proxies the Swift ipc/1 stream verbatim, then takes ownership source by source. There is exactly one authenticated external peer, durable command authority and authoritative producer for each source. A second peer evicts the first today.

At each step name who assigns capture IDs, spends credits, receives Committed and persists checkpoints. Native protocol results are paged with completeness evidence and file references for large payloads. Durable fences, tickets, fsynced guards and replay checks survive the language transition.
