# Apple Bridge for Mac

Apple Bridge connects agents such as Claude Code and Codex to your Mac’s Apple data through a local MCP server, with REST access for apps and bots.

**Early access · by invite.**

## Status and install

Early access is by invite from the repository maintainers. Public installation is not available yet: there is no public Mac installer or Homebrew package. The Mac release targets Apple Silicon and macOS 26 or later. Capabilities vary by source and macOS permissions; see the [feature status](features.yaml).

## Quick start: Level 1, local agents (default)

Install Apple Bridge on your Mac through the early-access invitation, then connect agents running on that same computer. The local MCP address is `http://127.0.0.1:7780/mcp`. No account, server or domain is needed. The relay is off unless you turn it on.

**Not yet tested with a real local app.** Authentication and tool use with a local agent still need a real-app test.

For invited testers who already have Apple Bridge installed:

1. Open Apple Bridge, start the gateway and grant the permissions for the sources you want to use, such as Messages, Calendar, Reminders, Contacts or Notes.
2. With Claude Code installed on that Mac, run this from the project where you want to use it:

   ```sh
   claude mcp add --transport http apple-bridge http://127.0.0.1:7780/mcp
   ```

   This is Claude Code’s HTTP MCP registration command; see its [MCP documentation](https://code.claude.com/docs/en/mcp). Other local agents, including Codex, use the same server address in their MCP settings.
3. Check the agent’s connection status and complete any requested authorization. Review the requested access before approving, then try reading one item from a source you enabled.

## Level 2: other apps and bots on this Mac

Other MCP-capable apps and bots running on the same computer use the same local address, `http://127.0.0.1:7780/mcp`. Developers can also use the local REST API. No account, server or domain is needed, and the relay can stay off. This route is also **not yet tested with a real local app**.

The local address only works on that Mac. ChatGPT and Claude’s websites cannot reach it.

## Level 3: remote access (optional)

Choose this level only to connect from another computer or from cloud apps such as ChatGPT and Claude’s website. Your Mac must stay awake and connected. There are two options:

- **Hosted relay (by invite):** on a Mac with Apple Bridge already installed and running, enable it explicitly:

  ```sh
  bridge remote on
  ```

  Follow the account-linking page using your invitation, then use the printed connector address ending in `/mcp` in your app’s custom MCP connector settings. The relay forwards traffic; TLS ends on your Mac.
- **Your own server (advanced):** run the relay on a Linux server with a domain you control. You still need Apple Bridge installed on your Mac. Follow [self-hosting](docs/self-host.md).

Both remote options require app authorization. See [connector setup and approvals](docs/connectors/approvals.md).

## How approvals protect you

Connecting through a relay does not grant an app access to your data. The connector request names the app and the access it wants; compare the matching code before approving on your Mac or paired iPhone. You can deny the request. Agents can have read-only, approval-required or full access: choose the least access your task needs. Approval-required writes wait for a separate decision; full access permits supported writes without that step.

Review [connector approvals](docs/connectors/approvals.md) before connecting an app. Report suspected vulnerabilities through the private route in [SECURITY.md](SECURITY.md).

## Documentation

- [Feature status and known limits](features.yaml)
- [Self-hosting](docs/self-host.md)
- [Connector approvals](docs/connectors/approvals.md)
- [Architecture decisions](docs/adr/)
- [Changelog](CHANGELOG.md)
- [Contributing and building from source](CONTRIBUTING.md)
- [MIT licence](LICENSE)
