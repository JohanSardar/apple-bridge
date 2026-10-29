# Connector qualification — 2026-10-29

- CONN-CHATGPT real: limited, needs connector login. The available browser session was signed out.
- CONN-CLAUDE-AI real: limited, needs connector login. The available browser session stopped at security verification.
- CONN-CLAUDE-CODE real: PASS. Claude loaded an isolated HTTP MCP configuration, called records_query against a synthetic gateway through the public relay, and returned a successful tool result. Temporary bearer credentials and gateway state were removed. Interactive OAuth consent remains an owner step; this result proves bearer-authenticated Code tool use.

The first Code attempt stopped at an unrelated external probe-directory failure. The connector runner now calls Claude directly through the public relay, without requiring the Ubuntu network qualification suite.

Required gate passed: make lint, make features, go build -p 4 ./..., go test -p 4 ./.... Native and web sources were unchanged. The warm native cache was cloned. GOFLAGS=-ldflags=-w avoided slow dsymutil work. Concurrent host load exceeded 200; compilation was reused, and no cold native build was run.

One sol-loop pass found that the new Code route also accepted non-real levels. The route is now restricted to real. No additional scenarios were run.
