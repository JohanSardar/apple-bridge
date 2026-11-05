# Apple Bridge for Mac

Apple Bridge gives AI apps and developers REST and MCP access to Apple data on a Mac, including Messages, Calendar, Reminders, Contacts and Notes.

**Early access · by invite.**

## Status and install

Early access is by invite from the repository maintainers. Public installation is not available yet: there is no public Mac installer or Homebrew package. The Mac release targets Apple Silicon and macOS 26 or later. Capabilities vary by source and macOS permissions; see the [feature status](features.yaml).

## Quick start

These steps are for invited testers who already have Apple Bridge installed and running on their Mac:

1. Open Apple Bridge and grant the permissions for the sources you want to use.
2. For the hosted relay, run this on that Mac:

   ```sh
   bridge remote on
   ```

   Follow the account-linking page using your invitation. Use the connector address it prints, ending in `/mcp`, in your app's custom MCP connector settings.
3. Compare the code shown by the connecting app with the approval request on your Mac. Check the requested access, then allow or deny it.
4. Ask the app to read one item from a source you enabled. Available operations depend on your permissions and the source's current support.

## How apps reach your Mac

Choose one of these routes:

- **Hosted relay (by invite):** for apps outside your Mac, including hosted MCP connectors. Your Mac must stay awake and connected. The relay forwards traffic; TLS ends on your Mac. See [connector setup and approvals](docs/connectors/approvals.md).
- **This Mac only (not yet tested with a real app):** no account or server is needed. Only apps running on that Mac can use the local REST/MCP endpoint. ChatGPT and Claude's websites cannot reach it. App-specific local setup is not yet qualified.
- **Your own server (advanced):** run the relay on a Linux server with a domain you control. You still need Apple Bridge installed on your Mac. Follow [self-hosting](docs/self-host.md).

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
