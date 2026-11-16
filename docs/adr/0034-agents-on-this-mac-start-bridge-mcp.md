# Agents on this Mac start `bridge mcp`

Amends how local agents connect in [ADR 0032](0032-approve-on-the-mac-without-a-code.md). The Allow click on the Mac stays.

An agent on this Mac connects by starting `bridge mcp`, a stdio MCP server, instead of signing in to `http://127.0.0.1:7780/mcp` through a browser. `bridge link <agent>` writes that command into the agent's own settings: through the agent's CLI for Claude Code, Codex, Gemini CLI and VS Code, by editing its JSON settings for Cursor, Zed, OpenCode and the Claude app, and through an install link for Goose. `bridge link other` prints the config for anything else.

`bridge mcp` answers `initialize` and `tools/list` at once from the gateway's own tool list, so the agent starts without waiting. If it has no key for this agent yet, it asks on the owner-only connector socket, as an agent request, and the Mac shows the Allow panel. Each request then waits for the decision, for at most 50 seconds, and goes to the gateway's `/mcp` with the key. Once allowed, the real tool list replaces the preview and the agent is told that the list changed.

The key is the `full` agent key from ADR 0032. It is cached in `local-agents/` in the home: 0600 files in a 0700 folder, one per agent. A cached key is named by the client's MCP name together with the outermost app bundle that started it, for example Claude Code in T3 Code, so a different agent, or the same agent in a different app, asks again. A revoked or expired key is dropped on the first 401, and the next request asks again.

The panel names the agent from what it tells us about itself (its `clientInfo` name) and the app found from its parent processes. Either can be faked by any program running as the owner. That is no weaker than before: such a program can already read the home or send a request to the connector socket. The owner still decides each new agent on the Mac, and `bridge agents revoke` ends one.

Grok Bot can't use this. Its Command connectors run on Grok's cloud computer and its Remote HTTPS connectors are called from Grok's servers; neither reaches a program on the Mac. `bridge link grok-bot` says so and changes nothing.
