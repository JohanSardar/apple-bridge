
G-D1/G-F reconciliation: OAuth uses `create_oauth_agent(name, level, scopes)`, which validates OAuth scopes and delegates to the shared typed `create_agent(name, AgentLevel, Vec<Grant>)`. CLI and OAuth share one admission/persistence implementation. G-F lifecycle modules and interfaces from PR #542 are retained.


## CORE pairing expiration precision

`Authority::pair_with_ttl` accepts a nonzero `std::time::Duration` up to ten minutes, retaining nanosecond precision in both code and persisted expiration. The CLI uses this method for `--expires-in`, including subsecond expirations. Existing `pair(..., ttl_seconds)` remains compatible and delegates to the duration method. Pairing code JSON and proof formats are unchanged.

## G-F current binary matrix

The assembled gateway now renders agent scopes and defaults an explicit empty level to approve. WEB-AGENT-CREDENTIALS still fails list_is_redacted after successful scoped requests: the CLI summary contains last_used only when Principal.extra supplies it; investigate authenticate_oauth's early acceptance of ordinary agent tokens before authenticate_token updates last_used. WEB-AGENT-LEVELS passes local owner approval execution/restart/expiry but delegated approval reauthorization and per-command activity remain unavailable. G-F retains failures rather than treating missing identity evidence as authorization.
