# Connector approvals

The OAuth authorization page says which app wants what, in product words, and asks the owner to approve on the Mac or paired iPhone. It shows the connector name, callback host and requested data. A small panel on the Mac shows the same request with Allow and Deny. A four-digit matching code appears on both only when the request could be confused with another: when more than one is waiting, or when it came through the public address without the owner starting a connection from this Mac in the last 15 minutes ([ADR 0032](../adr/0032-approve-on-the-mac-without-a-code.md)). Requests expire after five minutes. The random polling capability is separate from the matching code; neither a public HTTP request nor the four-digit code alone can approve a connector.

Allowing a request connects the app at level `full`: from then on its changes and actions run at once, with no further approval, including screenshots, recordings, camera photos, current location, Focus and Photos changes ([ADR 0030](../adr/0030-connected-apps-act-by-default.md)). The passkey page also offers "Read, and ask me before any change" and "Read only". To switch an app later, run `bridge agents update <id> --level approve` or `bridge agents update <id> --level full`.

There is one queue of requests per gateway, shared by every hostname, so a request arriving through the relay reaches the Mac and iPhone. The gateway opens the Helper's Allow and Deny window when a request starts waiting, and the window closes after a minute with nothing waiting. A paired iPhone receives the encrypted request through the existing push and pending-delivery transport, displays its code and scopes, and requires Face ID before sending a signed decision. The physical iPhone check belongs to I1.

`bridge link claude` and `bridge link codex` also watch this queue while they sign in, and print the request with the command that allows it. They never allow the request themselves. When Claude Code runs `bridge link claude` itself, it asks on the Mac instead of opening a browser: the panel shows "Claude Code wants to use Apple Bridge", and Allow gives Claude Code a key that expires in a year. Cloud agents on the owner's Tailscale network ask the same way through `/connect` on the Tailscale address.

The owner can also use the CLI on the gateway Mac:

```
bridge connectors list --home <gateway-home>
bridge connectors allow --request <id> --code <four-digit-code> --home <gateway-home>
bridge connectors deny --request <id> --code <four-digit-code> --home <gateway-home>
```

These commands use a Unix socket in a directory accessible only to the gateway's macOS account. Five failed code attempts refuse further decisions. Public creation is limited to ten requests a minute and one hundred a day per client address, with a global limit and at most 128 outstanding requests. Approval and redirect retrieval are each single use. Denial returns OAuth `access_denied`. A passkey is the fallback: once one is enrolled the page offers “Use a passkey instead”, which signs in and then offers Allow and Deny on the page.

## Reproduce A1

```
packaging/dev/rust-lab.sh sim lab/scenarios/core/CONN-CLAUDE-AI.yaml lab/scenarios/core/CONN-CHATGPT.yaml
ssh air '~/bin/ab-real A1 ship/a1-approve connector CONN-CLAUDE-AI CONN-CHATGPT'
```

Both sim scenarios prove an owner-local approval, OAuth redirect and token exchange, a successful `records_query` MCP call, the agent's requested scopes, denial, a wrong code, expiry, replay, separate concurrent requests and rate limits.

The real runner builds the current web UI, starts an isolated gateway through the public relay, and writes `~/Library/Application Support/AppleBridgeDev/connector-conn-claude-ai.json` or `connector-conn-chatgpt.json` on the test Mac. Each handoff contains `connector_url` and `approval_home`; it is removed when that scenario finishes. The runner approves matching Claude or ChatGPT requests through the owner-local Mac socket and waits for the connector to use its OAuth agent.

Within the real runner's five-minute handoff window, use Chrome on the test Mac, already signed in to the connector:

1. In Claude.ai Settings → Connectors, add the handoff's `connector_url` as a custom connector and connect. In ChatGPT Settings → Apps/Connectors, add the same scenario's custom MCP URL (enable developer mode if required).
2. Follow the OAuth page. It must display “Approve on your Mac or iPhone” and a matching four-digit code. The isolated runner completes the Mac approval itself; a packaged gateway instead opens Apple Bridge's Allow/Deny window.
3. In a new conversation, enable that connector and ask it to call `records_query` for one `messages.chat` record from the isolated bridge. Do not paste the records into evidence.
4. Remove the temporary connector after the check. The runner removes its handoff, disables its isolated relay and deletes its isolated state.

A browser that is signed out cannot complete either real scenario. The logged-in Chrome session on the test Mac must be driven by the orchestrator if it is unavailable to this agent.
