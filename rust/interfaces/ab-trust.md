
G-D1/G-F reconciliation: OAuth uses `create_oauth_agent(name, level, scopes)`, which validates OAuth scopes and delegates to the shared typed `create_agent(name, AgentLevel, Vec<Grant>)`. CLI and OAuth share one admission/persistence implementation. G-F lifecycle modules and interfaces from PR #542 are retained.


## CORE pairing expiration precision

`Authority::pair_with_ttl` accepts a nonzero `std::time::Duration` up to ten minutes, retaining nanosecond precision in both code and persisted expiration. The CLI uses this method for `--expires-in`, including subsecond expirations. Existing `pair(..., ttl_seconds)` remains compatible and delegates to the duration method. Pairing code JSON and proof formats are unchanged.
