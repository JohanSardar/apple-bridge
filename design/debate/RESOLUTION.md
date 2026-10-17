# Debate resolution (2026-10-16)

Rounds: Astra independent design → Claude r1 → Astra r1 → Claude r2 (this file). No point was left unresolved, so no Fable tiebreak was needed.

| # | Topic | Outcome |
|---|---|---|
| D1 | Mac process split | **Astra's design.** A thin Swift adapter process owns Apple access. A shared gateway/replica program owns the journal, persistence, REST and MCP. The gateway is pinned to the Mac in v1, with authenticated local IPC and the two Mac components shipped and upgraded together. |
| D2 | Shared service language | **Go** (Astra conceded). `modernc.org/sqlite`, provisionally. Self-contained executables per tested OS/arch. Memory, FTS5, WAL and packaging are benchmarked on the 8 GB Air as implementation gates. |
| D3 | Screen Time | **Mac-side experimental read-only collector** (knowledgeC/Biome), probed on Johan's exact macOS build with SIP on before any coverage is advertised. Records local-Mac vs iPhone origin, provenance and estimated durations. All phone paths removed (user scope). |
| D4 | Offline writes | **Fail fast by default** (Astra conceded). Delayed execution only on explicit opt-in with `expires_at` and per-operation max TTL, after v1. A lost response is *indeterminate*: recover by original command ID, never a fresh-ID resend. |
| D5 | Health | **No iPhone app or new phone setup** (user scope). Health only if a source already exists on the Mac, and then read-only with provenance. Otherwise advertised as unavailable. |
| D6 | Notes | After the core. Only demonstrated capabilities. Unsupported rich edits return explicit errors. |
| D7 | Contacts writes | **Claude concedes to Astra.** Basic contact create/update/delete ships in v1 alongside Contacts replication, using CNSaveRequest, the same command ledger, conflict checks and observed-result verification. It respects account writability. Group management comes later. |

Also binding: all 20 findings in `../claude/astra-critique-of-claude-r1.md`.
