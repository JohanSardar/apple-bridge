# Connecting other devices and cloud chats

Agents on your Mac use `http://127.0.0.1:7780/mcp` and need nothing else. This page is for everything else. The easiest place to do it is the Setup window: open Apple Bridge, then Setup > Open Setup, and go to Connect.

| You want to use Apple Bridge from | Use | Private? |
|---|---|---|
| Another computer of yours, Grok Bot | Tailscale | Yes. Only devices on your Tailscale network can reach your Mac |
| Claude or ChatGPT on the web | Cloud chats | Less. Your Mac gets a public address; only chats you approve get in |
| Your own server and domain | [Your own relay](../self-host.md) | You run the relay that forwards encrypted traffic |

## Tailscale: your other devices and Grok Bot

[Tailscale](https://tailscale.com) is a free private network. A device has to be signed in to your Tailscale account to reach your Mac, so nothing is on the public internet.

1. In Setup > Connect, click **Install Tailscale** if it isn't installed. Apple Bridge downloads Tailscale's installer from tailscale.com, checks it is signed by Tailscale, and opens it.
2. Open Tailscale and sign in.
3. Click **Turn On**. If your Tailscale network doesn't have HTTPS turned on yet, Setup shows **Turn On HTTPS**. Click it and choose Enable, once.

Setup then shows your Mac's Tailscale address, such as `https://your-mac.tail1234.ts.net/mcp`. In Terminal: `bridge remote enable --via tailnet`.

**Another computer.** Install Tailscale there and sign in with the same account. Then:

- Claude Code: `claude mcp add --transport http apple-bridge https://your-mac.tail1234.ts.net/mcp`, then `/mcp` to sign in.
- Codex: `codex mcp add apple-bridge --url https://your-mac.tail1234.ts.net/mcp && codex mcp login apple-bridge`.

A window on your Mac shows the app's name, what it asks for and a four-digit code. Click Allow if the code matches the sign-in page.

**Grok Bot.** Grok Bot's bots run on a cloud computer, so they can't use your Mac's local address. Its Tailscale plugin puts that computer on your Tailscale network:

1. In Grok Bot, open Connect apps, choose Tailscale, click Authenticate and sign in with your Tailscale account.
2. In Setup > Connect > Grok Bot, click **Make Key**. Or run `bridge agents create --name "Grok Bot" --tailnet --scope calendar:write,reminders:write --expires-in 90d`.
3. In Grok Bot, add a custom MCP server with your Mac's Tailscale address and the header `Authorization: Bearer <key>`.

The key only works at your Mac's Tailscale address, so it is useless on the public internet. Grok Bot uses a key rather than signing in because its sign-in for servers on a Tailscale network doesn't finish yet ([cursor/plugins#1108](https://github.com/cursor/plugins/issues/1108)).

To stop: **Turn Off** in Setup, or `bridge remote disable --via tailnet`.

## Cloud chats: Claude and ChatGPT

Claude and ChatGPT on the web run in the cloud and can't join Tailscale. To reach your Mac they need a public address, which Apple Bridge makes with [Tailscale Funnel](https://tailscale.com/kb/1223/funnel). Anyone can reach that address, and only chats you approve on your Mac get in. That is less private than Tailscale between your devices, so it is off until you choose it.

1. In Setup > Connect > Cloud chats, tick the services a cloud chat may see.
2. Click **Turn On Public Address**. The first time, Tailscale may ask you to allow Funnel for your network: click **Allow Funnel** and confirm.
3. Add the connector address Setup shows:
   - **Claude:** Customize > Connectors > Add > Add custom connector (Setup's **Open Claude** button opens it). Paste the address, click Add, then Connect.
   - **ChatGPT:** Settings > Apps > Advanced settings > Developer mode, then Create app. Paste the address and choose OAuth. Developer mode needs a paid plan.
4. A page opens with a four-digit code, and a window on your Mac shows the same code with what the chat can see. Click Allow on the Mac if they match.

A cloud chat is offered only the services you ticked, even if it asks for more. In Terminal: `bridge remote enable --via cloud --sources calendar,reminders,notes`.

To stop: **Disconnect and Close Address** in Setup, or `bridge remote disable --via cloud`. That closes the public address and ends the access of every chat that connected through it. Connecting again later needs a new approval.
