# Connecting Claude and ChatGPT

Run `bridge remote status` on your Mac. Your connector address is the URL it shows, followed by `/mcp`. Keep the Mac on and Apple Bridge running while you connect and use it.

**Claude:** Customize → Connectors → Add → Add custom connector. Give it a name, paste the address and choose Continue, then Add, then Connect.

**ChatGPT:** turn on Developer mode in ChatGPT's settings, then create an app with this address and OAuth. Developer mode needs a paid plan. OpenAI moves these settings often, and this path hasn't been walked with a real ChatGPT account yet.

**Claude Code:** `claude mcp add --transport http apple-bridge <address>`, then `/mcp` to sign in.

A page says which app wants what and shows a four-digit code. Approve on your Mac or paired iPhone if the code matches. See [connector approvals](approvals.md).

## What was proven with real clients

On 2026-11-04, with Claude on claude.ai and a gateway on a private copy of real data:

| Relay | Mac admitted by | Result |
|---|---|---|
| Hosted, `ab.lexaai.tech` | Account, by browser link | Connected, read real records |
| Self-hosted, with accounts | Account, by browser link | Connected, read real records |
| Self-hosted, no accounts, fresh install from the published release | Owner key | Connected, read real records |

Each read was checked against the database. ChatGPT has not been run for real yet: it is held to OpenAI's published requirements in `rust/crates/ab-frontdoor/tests/oauth_compliance.rs`, and `lab/compliance/` runs independent clients through a real relay.

## Checking a relay

```sh
cd lab/compliance && npm install
node oauth-clients.mjs https://your-mac-address path/to/approve-command
```

The approve command gets the request id and code as its two arguments. Run this against a real relay address, not only locally: some checks only apply on a public host, and that is where the worst bug in this area hid.
