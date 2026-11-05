# Apple Bridge for Mac

Apple Bridge connects agents such as Claude Code and Codex to your Mac’s Apple data through a local MCP server, with REST access for apps and bots.

**Early access · by invite.**

## Status and install

Early access is by invite from the repository maintainers. Public installation is not available yet: there is no public Mac installer or Homebrew package. The Mac release targets Apple Silicon and macOS 26 or later. Capabilities vary by source and macOS permissions; see the [feature status](features.yaml).

## Quick start: Level 1, local agents (default)

With Apple Bridge installed through your invitation, run this on your Mac:

```sh
bridge up                   # start Apple Bridge. It asks nothing
bridge link claude          # connect Claude Code. Or: bridge link codex
bridge permissions guide    # allow the Mac permissions you want
bridge stats --watch        # counts per source, as each one finishes
```

No account, server or domain is needed, and the relay stays off. Agents reach your Mac at `http://127.0.0.1:7780/mcp`.

`bridge link` signs in through your browser. Check the four-digit code on the page, then allow it in the Apple Bridge window or with `bridge connector allow`. Allowed agents can read, and every change waits for your approval.

Then ask your agent: "Use Apple Bridge to show my reminder lists. Then run `bridge stats` and tell me how much has synced."

To try things yourself, use `bridge read reminders`, `bridge read chats` or `bridge search "dinner"`. To teach an agent, tell it to run `bridge skills get core`. [Setting up your Mac](docs/local-setup.md) covers every command, and `bridge --help` lists them.

**What has been tested.** On a test copy of real data, a fresh home reached a real Claude Code read in about 13 seconds through the local MCP address and sign-in, and Codex completed sign-in the same way. A script played the browser and the approval. The Apple Bridge window, real macOS permission prompts and a real first sync from Apple's apps were not part of that test. See [what was proven](docs/local-setup.md#what-has-been-proven).

## Level 2: other apps and bots on this Mac

Other MCP-capable apps and bots on the same computer use the same local address. `bridge link other` prints what they need, including how to create a read-only token with `bridge agent create`. No account, server or domain is needed, and the relay can stay off. Developers can also use the local REST API. This route is **not yet tested with a real app** other than Claude Code and Codex.

The local address only works on that Mac. ChatGPT and Claude's websites cannot reach it.

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

- [Setting up your Mac](docs/local-setup.md)
- [Feature status and known limits](features.yaml)
- [Operations reference](docs/reference/operations.md)
- [Self-hosting](docs/self-host.md)
- [Connector approvals](docs/connectors/approvals.md)
- [Architecture decisions](docs/adr/)
- [Changelog](CHANGELOG.md)
- [Contributing and building from source](CONTRIBUTING.md)
- [MIT licence](LICENSE)
