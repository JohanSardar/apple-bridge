# Connector approvals

An unauthenticated OAuth authorization page asks the owner to approve on the Mac or paired iPhone. It shows the connector name, callback host, exact requested scopes and a four-digit matching code. Requests expire after five minutes. The random polling capability is separate from the matching code; neither a public HTTP request nor the four-digit code alone can approve a connector.

The Mac app displays Allow and Deny. The gateway launches its approval window when needed. A paired iPhone receives the encrypted request through the existing push and pending-delivery transport, displays its code and scopes, and requires Face ID before sending a signed decision. The physical iPhone check belongs to I1.

The owner can also use the CLI on the gateway Mac:

```
bridge connector pending --home <gateway-home>
bridge connector allow --request <id> --code <four-digit-code> --home <gateway-home>
bridge connector deny --request <id> --code <four-digit-code> --home <gateway-home>
```

These commands use a Unix socket in a directory accessible only to the gateway's macOS account. Five failed code attempts refuse further decisions. Public creation is limited to ten requests a minute and one hundred a day per client address, with a global limit and at most 128 outstanding requests. Approval and redirect retrieval are each single use. Denial returns OAuth `access_denied`. The existing passkey consent page remains available through “Use a passkey instead”.

## Reproduce A1

```
go run ./cmd/bridge-lab run lab/scenarios/core/CONN-CLAUDE-AI.yaml --level sim
go run ./cmd/bridge-lab run lab/scenarios/core/CONN-CHATGPT.yaml --level sim
ssh air '~/bin/ab-real A1 ship/a1-approve connector CONN-CLAUDE-AI CONN-CHATGPT'
```

Both sim scenarios prove an owner-local approval, OAuth redirect and token exchange, a successful `records_query` MCP call, the agent's requested scopes, denial, a wrong code, expiry, replay, separate concurrent requests and rate limits.

The real runner builds the current web UI, starts an isolated gateway through the public relay, and writes `~/Library/Application Support/AppleBridgeDev/connector-conn-claude-ai.json` or `connector-conn-chatgpt.json` on the Air. Each handoff contains `connector_url` and `approval_home`; it is removed when that scenario finishes. The runner approves matching Claude or ChatGPT requests through the owner-local Mac socket and waits for the connector to use its OAuth agent.

Within the real runner's five-minute handoff window, use Chrome on the Air, already signed in to the connector:

1. In Claude.ai Settings → Connectors, add the handoff's `connector_url` as a custom connector and connect. In ChatGPT Settings → Apps/Connectors, add the same scenario's custom MCP URL (enable developer mode if required).
2. Follow the OAuth page. It must display “Approve on your Mac or iPhone” and a matching four-digit code. The isolated runner completes the Mac approval itself; a packaged gateway instead opens Apple Bridge's Allow/Deny window.
3. In a new conversation, enable that connector and ask it to call `records_query` for one `messages.chat` record from the isolated bridge. Do not paste the records into evidence.
4. Remove the temporary connector after the check. The runner removes its handoff, disables its isolated relay and deletes its isolated state.

A browser that is signed out cannot complete either real scenario. The logged-in Chrome session on the Air must be driven by the orchestrator if it is unavailable to this agent.
