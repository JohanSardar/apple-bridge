# The browser gets its own front door with passkey device sessions

The REST/MCP Frontdoor rejects every browser origin on purpose, and bearer tokens in browser storage would be exposed to injected script. So browsers use a separate Web module on the same origin. A passkey sign-in creates a `device:` principal held in an HttpOnly, SameSite=Strict cookie. Every state change must carry its own Origin header. Commands from device sessions are interactive, which means the person at the keyboard can text anyone. Agent and lab principals keep their own gates.
