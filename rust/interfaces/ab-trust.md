
G-D1/G-F reconciliation: OAuth uses `create_oauth_agent(name, level, scopes)`, which validates OAuth scopes and delegates to the shared typed `create_agent(name, AgentLevel, Vec<Grant>)`. CLI and OAuth share one admission/persistence implementation. G-F lifecycle modules and interfaces from PR #542 are retained.

