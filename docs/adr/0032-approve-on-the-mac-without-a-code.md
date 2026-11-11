# Approve on the Mac without a matching code

Amends the connecting rule in [ADR 0030](0030-connected-apps-act-by-default.md).

A new app still connects only after the owner clicks Allow on the Mac (or a paired iPhone, or a passkey). What changes is the four-digit code. The sign-in page and the Mac panel no longer show it when the request cannot be confused with another:

- the request came from this Mac (`http://127.0.0.1`), from a device on the owner's Tailscale network, or through the public address within 15 minutes of `bridge link claude-app` on this Mac, which writes `remote/connect-armed`; and
- it is the only request waiting.

Otherwise both sides show the code, as before, and it stays shown for that request. The code is still generated and still checked when a decision is made; only its display changed. The Mac panel says where the request came from ("An app on this Mac", "A device on your Tailscale network", or the host it returns to).

A second kind of request, an agent request, asks for a bearer key instead of an OAuth code. Only two callers can create one: the owner-only connector socket on the Mac, which `bridge link claude` uses when Claude Code runs it, and `POST /connect/request` on the Tailscale route, which cloud agents such as Grok Bot use. Neither is reachable from the public address. Allowing one creates a `full` agent that expires in a year; a Tailscale one is bound to the Tailscale address. The requester collects the key once, with a secret returned when it asked.

The owner asked for this on 2026-11-11: comparing a code made connecting feel unsafe and amateur. What we give up is protection against an unexpected request arriving while the owner expects their own: an attacker who knows the public address and sends a request in the armed window, before the owner's own request, would get a panel without a code. The panel still names the returning host, a second request turns the code back on, and the window is short.
