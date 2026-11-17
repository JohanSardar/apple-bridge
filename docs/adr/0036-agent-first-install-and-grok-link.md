# An agent can finish setup from the install link

The canonical install script points agents directly to `https://eilmer.si/applebridge/setup.md`. That document is staged from the setup instructions embedded in the binary. Setup and Markdown reference docs are served on the installer host, without depending on repository rendering or API limits. Current source and issue links point to GitHub.

`bridge link grok-bot --json`, running on the Mac, starts the private route and requests an agent through the existing owner-only connector socket. The request is labelled Grok Bot and originates on this Mac. The socket derives its MCP resource from the active Tailscale route; the caller cannot supply an arbitrary resource. The request uses the same Allow panel and five-minute, secret-protected, single-use collection as other agent requests. The command performs the waiting, so a cloud computer need not already reach the tailnet to collect its connector config. Denial, expiry and gateway restart fail without returning a key. The key is valid for the private MCP resource only, as in ADR 0035.

This extends ADR 0032: a request originating on this Mac may request a key bound to the Tailscale resource. It changes neither the Allow requirement nor the permitted callers. The only access to this option is the owner-only local socket. The HTTPS request endpoint remains available only through the private Tailscale route.

The agent configures its own connector with the returned URL and Authorization header. Grok Bot must use its Tailscale-connected cloud computer; a public HTTPS connector outside that computer cannot reach the private route. The setup guide distinguishes Mac execution access, connector access and identity sign-in, and requires a real MCP read before claiming connection. It forbids automatic Tailscale ACL or sleep-policy changes. The installer cannot grant execution access to a Mac or complete an identity sign-in for the owner.

No new service, relay, public endpoint or account is introduced. Existing connected agents and native permissions are preserved.
