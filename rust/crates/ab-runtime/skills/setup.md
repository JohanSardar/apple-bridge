# Connect to Apple Bridge

If an agent is given https://eilmer.si/applebridge/install and told to connect, follow this guide. Do the work yourself with the tools you have. The owner only handles identity sign-in and Mac consent and permissions. Never click Allow, grant a permission, type a password, change Tailscale access rules or approve a request for them. Never ask them to copy a key or run commands when your tools can do it.

## 1. Find where you run

- On the owner's Apple silicon Mac with macOS 26 or later: install and link locally.
- Grok Bot: your connectors run on Grok's cloud computer, even if you can execute commands on the Mac. The connector computer must join the owner's private Tailscale network through Grok Bot's Tailscale plugin. A local Mac URL or Mac stdio command will not work there. Configure the connector on that cloud computer using the private HTTPS URL and bearer header below.
- Another computer: use its existing authorized SSH connection to the Mac to install and run the Mac steps. Connect over private Tailscale, or register `ssh -T <owner>@<mac> ~/.applebridge/bin/bridge mcp` as your stdio MCP command.

Use existing authorized Mac access. If you have no tool that can run on the Mac, say exactly that and ask for that access. A public link cannot reach or install software on a Mac by itself. If Tailscale or its plugin needs identity sign-in, the owner must complete that sign-in; never handle their password. Do not create a relay, public route, hosted service or new account for Apple Bridge.

## 2. Install and start on the Mac

```sh
curl -fsSL https://eilmer.si/applebridge/install | sh
~/.local/bin/bridge up
```

The installer needs no Node, Homebrew or sudo. Use the absolute path in the current shell: a PATH change only takes effect in new shells. If already installed, check `bridge --version` and use the current release. Updating stops the previous gateway, so preserve existing agents and permissions and never reinstall an unrelated live test target.

## 3. Connect

### Agent on the Mac

```sh
~/.local/bin/bridge link claude
```

Use your own client name: `codex`, `cursor`, `gemini`, `vscode`, `zed`, `goose`, `opencode`, or `claude-app`. For any other client, `bridge link other` prints the MCP config. Reload your MCP configuration, then call `apple_status` (the `bridge.list_sources` operation). The first tool call asks the owner once through the Mac's Allow panel. Say: "Click Allow on your Mac." Never approve it yourself.

If a link command returns `terminal_required` with `outcome: pending`, use `bridge link <client> --print` for offline configuration instructions or hand the terminal step to the owner. Do not repeat the same headless command.

### Grok Bot, from a local terminal on the Mac

```sh
~/.local/bin/bridge link grok-bot --json
```

The command starts Apple Bridge if needed, turns on private Tailscale access and waits for the owner to click Allow. Run it in a local terminal on the Mac. Without one, it exits immediately with `terminal_required` and `outcome: pending`. `--timeout 2m` bounds the Allow wait. A denial or expiry fails without giving you a key. `--print` shows the command without starting anything or asking.

After Allow, stdout is JSON with `mcp`, `header`, `token`, and a ready `config`. These are credentials: keep them out of chat, logs, screenshots and source control. Use your Grok Bot connector tools to configure a streamable HTTP MCP connection on its Tailscale-connected cloud computer, with the returned `mcp` URL and the returned Authorization header on every request. Do not register it through a public HTTPS connector whose requests come from outside that computer. Do not paste the key back to the owner. Joining Tailscale gives network reach only; Allow gives access.

### Remote agent already on the tailnet

In a local terminal on the Mac run `bridge tailscale`; its output gives a private `/connect` URL. Read that URL from the connector's own computer. It returns machine-readable instructions: POST a short app name to `/connect/request`, then POST the returned id and secret to `/connect/answer` every three seconds until allowed or denied. After Allow the answer returns the MCP URL and header once. Store and configure them privately. If the connector cannot yet reach the tailnet, use the Mac command above for Grok Bot instead of trying to poll from an unreachable computer.

`bridge tailscale` reports devices blocked by the owner's rules. Never change those rules. Tell the owner which device and port are blocked. Do not enable Funnel or expose a public port. The Mac must stay awake and online; do not change system sleep settings automatically.

## 4. Permissions and proof

Run `bridge permissions status --json`, which never prompts. For a requested source, the owner can run `bridge permissions request <name>` or `bridge permissions guide --only <names>` in a local terminal on the Mac. The guide refreshes the Helper after changed permissions. A headless or `--non-interactive` invocation returns `terminal_required` and leaves permissions pending. Use the status command to check again after the owner acts.

From your actual configured MCP client, call `apple_status` (the `bridge.list_sources` operation), then a harmless read of a permitted source the owner asked to use. Report a real result or the exact permission or reachability failure. `bridge stats --wait --timeout 2m` shows sync counts. A `partial` or `failed` sync requires checking the affected sources before claiming success. Retry a safe read after a permission refresh. For a partially applied or indeterminate write, inspect its request ID and affected records before further changes.

For a failure: `bridge status --json`, `bridge doctor`, and `bridge permissions status --json` distinguish gateway health, the private route and permissions. Keep private data and credentials out of diagnostics. `bridge up` is detached from the terminal but does not restart after logout, reboot or a crash; the app's Launch at Login is a separate owner choice. A network change should reconnect the Tailscale route while the gateway stays up. Do not promise uptime when the Mac is asleep or offline.

## Reference

- https://eilmer.si/applebridge/docs/local-setup.md
- https://eilmer.si/applebridge/docs/connectors/devices.md
- https://eilmer.si/applebridge/docs/reference/cli.md
- https://github.com/JohanSardar/apple-bridge
