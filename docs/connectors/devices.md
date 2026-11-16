# Use Apple Bridge from another computer, a server or Grok Bot

Agents on your Mac connect with `bridge link <agent>` ([connect an agent](../local-setup.md#connect-an-agent)). Anything that isn't on your Mac uses a private network: your Tailscale network, or plain SSH. Nothing is ever on the public internet, and every new agent still needs you to click Allow once on the Mac ([ADR 0035](../adr/0035-agents-off-the-mac-use-a-private-network-and-a-key.md)).

The Mac has to be awake. `bridge tailscale --keep-awake` keeps it awake with the lid closed; macOS asks for your password or Touch ID.

## Over Tailscale: servers, other computers and Grok Bot

On the Mac, run:

```sh
bridge tailscale
```

It installs and starts Tailscale if needed, turns on the Tailscale route (Tailscale Serve, on your Tailscale network only) and prints one sentence:

```text
Connect to Apple Bridge at https://your-mac.tail1234.ts.net:8443/connect and use it from now on.
```

Give that sentence to the agent: Claude Code or Codex on your VPS, an agent on another computer, or your bot in Grok Bot. The agent asks your Mac, a panel names it and the device it asked from, for example "Claude Code on my-vps", and you click Allow. The agent gets its own key and sets itself up; it works from then on. Nothing on its side needs a browser, so a headless server is fine.

What the agent does, if you want to check or do it by hand:

1. `POST {"name": "Claude Code"}` to `/connect/request`. It gets an `id` and a `secret`.
2. `POST {"id": ..., "secret": ...}` to `/connect/answer` every few seconds until you click Allow.
3. The answer has the `token`, the `mcp` address and the `header`. For example: `claude mcp add --transport http apple-bridge https://your-mac.tail1234.ts.net:8443/mcp --header "Authorization: Bearer <token>"`.

**Grok Bot.** Grok Bot's bots run on Grok's cloud computer, and its Tailscale plugin puts that computer on your Tailscale network. Connect the plugin once, then give your bot the sentence above.

To make a key yourself instead: `bridge agents create --tailnet`. To stop: `bridge tailscale --off`. To end one agent: `bridge agents revoke <id>`.

### When your Tailscale rules block a device

Your Tailscale access rules decide which of your devices can reach which ports. If they don't allow a device to reach the Mac on the route's port (8443, or 9443 or 4443 if that's taken), its connection waits and never gets an answer. `bridge tailscale` checks your rules each time and says so:

```text
! Your Tailscale access rules don't let my-vps reach this Mac on port 8443. A connection from there waits
  and then fails without an answer.
  To let them in, add this rule to "acls" in your Tailscale policy file at
  https://login.tailscale.com/admin/acls/file, then run bridge tailscale again to check:
    {"action": "accept", "src": ["100.101.1.30"], "dst": ["100.101.1.10:8443"]}
```

Apple Bridge never changes your rules. A device that joins later, such as Grok Bot's cloud computer, shows up the next time you run `bridge tailscale`. If a device can already reach the Mac over SSH, it says so and prints the SSH command below.

## Over plain SSH

If the server can SSH into your Mac (Remote Login on, and its key in your `~/.ssh/authorized_keys`), its agent can start Apple Bridge on the Mac over SSH. Use this as the agent's MCP command:

```sh
ssh -T you@your-mac ~/.applebridge/bin/bridge mcp
```

For example, in an MCP config: `{"command": "ssh", "args": ["-T", "you@your-mac", "~/.applebridge/bin/bridge mcp"]}`. The first time, the panel names it, for example "Claude Code over SSH from my-vps", and you click Allow. This works over any network that carries SSH and doesn't use the Tailscale route.

## Your iPhone

`bridge pair --iphone --via-remote` pairs the Apple Bridge iPhone app over the Tailscale route.
