# Approve on the Mac without a matching code

Amends the connecting rule in [ADR 0030](0030-connected-apps-act-by-default.md).

A new app still connects only after the owner clicks Allow on the Mac (or a paired iPhone, or a passkey). What changes is the four-digit code. The sign-in page and the Mac panel no longer show it when the request cannot be confused with another:

- the request came from this Mac (`http://127.0.0.1`) or from a device on the owner's Tailscale network; and
- it is the only request waiting.

Otherwise both sides show the code, as before, and it stays shown for that request. The code is still generated and still checked when a decision is made; only its display changed. The Mac panel says where the request came from ("An app on this Mac" or "A device on your Tailscale network").

A second kind of request, an agent request, asks for a bearer key instead of an OAuth code. Only two callers can create one: the owner-only connector socket on the Mac, which `bridge link claude` uses when Claude Code runs it, and `POST /connect/request` on the Tailscale route, which cloud agents such as Grok Bot use. Allowing one creates a `full` agent that expires in a year; a Tailscale one is bound to the Tailscale address. The requester collects the key once, with a secret returned when it asked.

The owner asked for this on 2026-11-11: comparing a code made connecting feel unsafe and amateur. What we give up is protection against an unexpected request arriving while the owner expects their own: another app on the Mac or a device on the tailnet that asks first would get a panel without a code. The panel still names where the request came from, and a second request turns the code back on.
