# Connecting other devices and cloud chats

Agents on your Mac use `http://127.0.0.1:7780/mcp` and need nothing else. This page is for everything else. Each way is one command on the Mac, and every new connection still needs you to click Allow in the small Apple Bridge window.

| You want to use Apple Bridge from | Run on the Mac | Private? |
|---|---|---|
| Another computer of yours, Grok Bot | `bridge tailscale` | Yes. Only devices on your Tailscale network can reach your Mac |
| The Claude app or claude.ai | `bridge link claude-app` | Less. Your Mac gets a public address; only chats you allow get in |
| Your own server and domain | `bridge relay setup` ([your own relay](../self-host.md)) | You run the relay that forwards encrypted traffic |

The Mac has to be awake for any of them, including when a cloud agent uses it while you're away.

## Tailscale: your other devices and Grok Bot

[Tailscale](https://tailscale.com) is a free private network. A device has to be signed in to your Tailscale account to reach your Mac, so nothing is on the public internet.

```sh
bridge tailscale
```

It installs Tailscale if it's missing (from tailscale.com, checking that the installer is signed by Tailscale), starts it, signs it in through your browser if needed, and turns on the Tailscale route. It prints two addresses and copies the second:

- `https://your-mac.tail1234.ts.net/mcp` for apps that sign in, such as Claude Code or Codex on another computer: `claude mcp add --transport http apple-bridge https://your-mac.tail1234.ts.net/mcp`.
- `https://your-mac.tail1234.ts.net/connect` for cloud agents.

**Grok Bot.** Grok Bot's bots run on a cloud computer, and its Tailscale plugin puts that computer on your Tailscale network. In Grok Bot, connect the Tailscale plugin once, then tell your bot: "connect to Apple Bridge at https://your-mac.tail1234.ts.net/connect and use it from now on". The bot asks your Mac, a panel says "Grok Bot wants to use Apple Bridge", and you click Allow. The bot gets a key that only works on your Tailscale network. It uses a key because Grok Bot's sign-in for servers on a Tailscale network doesn't finish yet ([cursor/plugins#1108](https://github.com/cursor/plugins/issues/1108)).

To stop: `bridge tailscale --off`.

## Claude and other cloud chats

Claude on the web and in the Claude app run in the cloud and can't join Tailscale. To reach your Mac they need a public address, which Apple Bridge makes with [Tailscale Funnel](https://tailscale.com/kb/1223/funnel). Anyone can reach that address, and only chats you allow on your Mac get in.

```sh
bridge link claude-app
```

It turns the public address on (installing and starting Tailscale if needed), copies the connector address and opens Claude. In Claude: Customize > Connectors > Add custom connector. Name it Apple Bridge, paste the address, click Add, then Connect. A sign-in page says "Approve on your Mac", and the Apple Bridge panel asks you to Allow. For 15 minutes after this command the panel needs no matching code; a request at any other time, or two at once, shows a code on both sides ([why](../adr/0032-approve-on-the-mac-without-a-code.md)).

To limit what cloud chats can see: `bridge remote enable --via cloud --sources calendar,reminders,notes`. ChatGPT works the same way through Settings > Apps > Advanced settings > Developer mode > Create app, with OAuth.

To stop: `bridge remote disable --via cloud`. That closes the public address and ends every cloud chat's access.
