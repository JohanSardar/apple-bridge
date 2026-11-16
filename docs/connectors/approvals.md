# Connector approvals

The OAuth authorization page says which app wants what, in product words, and asks the owner to approve on the Mac or paired iPhone. It shows the connector name, callback host and requested data. A small panel on the Mac shows the same request with Allow and Deny. A four-digit matching code appears on both only when the request could be confused with another: when more than one is waiting, or when it came from somewhere other than this Mac or the owner's Tailscale network ([ADR 0032](../adr/0032-approve-on-the-mac-without-a-code.md)). Requests expire after five minutes. The random polling capability is separate from the matching code; neither an HTTP request nor the four-digit code alone can approve a connector.

Allowing a request connects the app at level `full`: from then on its changes and actions run at once, with no further approval, including screenshots, recordings, camera photos, current location, Focus and Photos changes ([ADR 0030](../adr/0030-connected-apps-act-by-default.md)). The passkey page also offers "Read, and ask me before any change" and "Read only". To switch an app later, run `bridge agents update <id> --level approve` or `bridge agents update <id> --level full`.

There is one queue of requests per gateway, shared by every hostname, so a request arriving through the Tailscale route reaches the Mac and iPhone. The gateway opens the Helper's Allow and Deny window when a request starts waiting, and the window closes after a minute with nothing waiting. A paired iPhone receives the encrypted request through its paired connection, displays its code and scopes, and requires Face ID before sending a signed decision. The physical iPhone check belongs to I1.

Agents on this Mac start `bridge mcp`, which `bridge link` sets up ([ADR 0034](../adr/0034-agents-on-this-mac-start-bridge-mcp.md)). The first time an agent starts it, `bridge mcp` asks on the Mac without a browser: the panel names the agent and the app it runs in, such as "Claude Code in Terminal", and Allow gives that agent a key that expires in a year. The key is kept in `local-agents/` in the home, so the next start doesn't ask. `bridge mcp` never allows a request itself. Agents that aren't on the Mac ask the same way through `/connect` on the Tailscale address, and the panel names the device they asked from, such as "Claude Code on my-vps", from Tailscale's own record of the source address. An agent that starts `bridge mcp` over SSH is named by where the SSH connection came from, such as "Claude Code over SSH from my-vps" ([ADR 0035](../adr/0035-agents-off-the-mac-use-a-private-network-and-a-key.md)). Allow gives either one read and change access.

The owner can also use the CLI on the gateway Mac:

```
bridge connectors list --home <gateway-home>
bridge connectors allow --request <id> --code <four-digit-code> --home <gateway-home>
bridge connectors deny --request <id> --code <four-digit-code> --home <gateway-home>
```

These commands use a Unix socket in a directory accessible only to the gateway's macOS account. Five failed code attempts refuse further decisions. Request creation is limited to ten requests a minute and one hundred a day per client address, with a global limit and at most 128 outstanding requests. Approval and redirect retrieval are each single use. Denial returns OAuth `access_denied`. A passkey is the fallback: once one is enrolled the page offers “Use a passkey instead”, which signs in and then offers Allow and Deny on the page.
