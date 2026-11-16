# Connecting other devices

Agents on your Mac connect with `bridge link <agent>`, which has them start `bridge mcp` ([connect an agent](../local-setup.md#connect-an-agent)). This page is for your other devices. They reach your Mac through [Tailscale](https://tailscale.com), a free private network, and every new connection still needs you to click Allow in the small Apple Bridge window.

The Mac has to be awake, including when a cloud agent uses it while you're away. `bridge tailscale --keep-awake` keeps it awake with the lid closed; macOS asks for your password or Touch ID.

## Tailscale: your other devices and Grok Bot

A device has to be signed in to your Tailscale account to reach your Mac, so nothing is on the public internet.

```sh
bridge tailscale
```

It installs Tailscale if it's missing (from tailscale.com, checking that the installer is signed by Tailscale), starts it, signs it in through your browser if needed, and turns on the Tailscale route. The route uses Tailscale Serve, so only devices on your Tailscale network can reach it. It prints two addresses and copies the second:

- `https://your-mac.tail1234.ts.net/mcp` for apps that sign in, such as Claude Code or Codex on another computer: `claude mcp add --transport http apple-bridge https://your-mac.tail1234.ts.net/mcp`.
- `https://your-mac.tail1234.ts.net/connect` for cloud agents.

**Grok Bot.** Grok Bot's bots run on a cloud computer, and its Tailscale plugin puts that computer on your Tailscale network. In Grok Bot, connect the Tailscale plugin once, then tell your bot: "connect to Apple Bridge at https://your-mac.tail1234.ts.net/connect and use it from now on". The bot asks your Mac, a panel says "Grok Bot wants to use Apple Bridge", and you click Allow. The bot gets a key that only works on your Tailscale network. It uses a key because Grok Bot's sign-in for servers on a Tailscale network doesn't finish yet ([cursor/plugins#1108](https://github.com/cursor/plugins/issues/1108)).

To make such a key yourself instead: `bridge agents create --tailnet`.

To stop: `bridge tailscale --off`.

## Your iPhone

`bridge pair --iphone --via-remote` pairs the Apple Bridge iPhone app over the same Tailscale route.
