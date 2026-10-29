# Relay connectors

Use the gateway's `bridge remote status` URL followed by `/mcp` as the connector endpoint. The public relay preserves the gateway's OAuth discovery and authorization endpoints. Keep the gateway running while connecting and calling tools.

In ChatGPT, sign in, enable developer mode in settings if required by the account, and create a custom MCP connector using that endpoint and OAuth. In Claude web, sign in and add a custom connector from Settings > Connectors with the same endpoint. Complete the bridge authorization screen using the owner session and approve only the intended read scopes. Login approval remains pending until Johan completes it; relay reachability alone does not count as connector approval.

In Claude Code, register an HTTP MCP server with `claude mcp add --transport http apple-bridge <endpoint>`, then use `/mcp` to authenticate and approve the bridge login. Ask the client to call `records_query` with `limit: 1` against tagged test data. Confirm a successful tool result, rather than a text response claiming success. Do not save returned personal records, authorization codes, or credentials as evidence.

Run each qualification separately:

```
go run ./cmd/bridge-lab run lab/scenarios/core/CONN-CHATGPT.yaml --level real
go run ./cmd/bridge-lab run lab/scenarios/core/CONN-CLAUDE-AI.yaml --level real
go run ./cmd/bridge-lab run lab/scenarios/core/CONN-CLAUDE-CODE.yaml --level real
```

The web scenarios currently report `limited: needs connector login`: the available browser is signed out of ChatGPT and Claude web requires security verification. They do not claim an approved OAuth grant or successful web tool call. Claude Code qualification creates a temporary synthetic gateway through the public relay, loads an isolated bearer-authenticated MCP connector configuration, and requires an actual Claude tool-use event and successful tool result. Its temporary credentials and gateway are cleaned up. This proves the Code transport and tool call, while interactive OAuth approval is a separate owner step.
