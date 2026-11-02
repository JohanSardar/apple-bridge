# Consent front door with passkey sessions

REST/MCP reject browser origins. A minimal same-origin consent and approval front door uses passkey device sessions in HttpOnly SameSite=Strict cookies. Every state change requires its serving Origin. Browser read models and live sockets are removed; read models remain REST/MCP operations.
