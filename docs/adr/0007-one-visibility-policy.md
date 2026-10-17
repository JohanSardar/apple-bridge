# One visibility policy

A single Trust visibility function filters snapshots, changes, queries, search, blobs, commands, command results, SSE and MCP. Replica identities are distinct from scoped API tokens, and forwarded commands carry a signed delegated identity. Local file access on a replica bypasses API scopes and revocation cannot erase copied data; this is documented rather than hidden.
