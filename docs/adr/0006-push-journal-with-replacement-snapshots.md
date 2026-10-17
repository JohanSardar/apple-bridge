# Push journal with replacement snapshots

Replicas follow the gateway journal over a WebSocket feed with `(after, through]` batches, credits and acknowledgments; they never poll. Bootstrap and recovery use a snapshot frozen at sequence S, staged and swapped atomically with the cursor, followed by replay from S+1 out of the same journal. NATS/PowerSync/Electric were rejected because the sequencing, scoped filtering and snapshot semantics would still be ours and they add services on an 8 GB Mac.
