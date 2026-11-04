# Run your own relay

The relay lets Claude, ChatGPT and your iPhone reach your Mac from anywhere. It forwards encrypted traffic to your Mac, and the TLS keys stay on your Mac.

You can use the hosted relay on `lexaai.tech`, with an invite, or run your own by following this guide. It takes about ten minutes.

## What you need

- A small Linux server with a public IPv4 address. 1 GB of memory is plenty. x86_64 only for now.
- A domain, or a subdomain of one you control, used only for this. The examples use `bridge.example.com`.
- Your Mac with Apple Bridge installed.

## 1. Get your Mac's key

On your Mac:

```sh
bridge remote key --relay-url https://bridge.example.com
```

It prints one line. That key is what lets this Mac use your relay. Nothing else can.

## 2. Point your domain at the server

Where your domain's DNS is hosted, add two records. `203.0.113.10` stands for your server's IP.

| Name | Type | Value |
|---|---|---|
| `ns1.bridge.example.com` | A | `203.0.113.10` |
| `bridge.example.com` | NS | `ns1.bridge.example.com` |

Your relay answers DNS for `bridge.example.com` itself. That is how each Mac gets its own address and certificate, and it means the relay needs no API token for your DNS provider.

Open these on your server's firewall: TCP 443, TCP 53 and UDP 53.

## 3. Install

On the server, as root:

```sh
curl -fsSL https://raw.githubusercontent.com/JohanSardar/apple-bridge/main/deploy/selfhost/install.sh -o install.sh
bash install.sh --domain bridge.example.com --owner-key YOUR_MAC_KEY
```

The first start asks Let's Encrypt for a certificate, which needs step 2 to be live. If DNS hasn't spread yet, the service keeps retrying on its own. To check it:

```sh
ab-relay-admin status
curl https://bridge.example.com/info
```

## 4. Connect your Mac

```sh
bridge remote on --relay-url https://bridge.example.com
```

It prints your address, for example `https://8f3a….bridge.example.com/mcp`. Your Mac has its own certificate for it, and your relay only ever sees encrypted traffic.

## 5. Connect an app

**Claude:** Settings → Connectors (or Customize → Connectors) → Add custom connector. Paste your address, ending in `/mcp`.

**ChatGPT:** Settings → Apps → Advanced settings → turn on Developer mode, then Create app. Paste the same address and choose OAuth. Developer mode needs a paid ChatGPT plan.

A page opens saying which app wants what, with a four-digit code. Your Mac shows the same code in an Allow or Deny window. Check that the codes match, then choose Allow. Your iPhone can approve too, if it's paired.

## Letting other people use your relay

Add accounts, so people can link their own Macs by invite:

```sh
bash install.sh --domain bridge.example.com --accounts
ab-relay-admin invite --note "Sam"
```

Send them the link it prints. They create an account at `app.bridge.example.com`, then run `bridge remote on --relay-url https://bridge.example.com`, and a browser page asks them to confirm their Mac.

An account can link Macs and turn tunnels off. It can't reach anyone's data, and that includes you as the relay's operator: each Mac still approves its own connections.

## Everyday commands

| Command | What it does |
|---|---|
| `ab-relay-admin status` | Service health |
| `ab-relay-admin add-key KEY` | Allow another Mac by key, with no restart |
| `ab-relay-admin invite [--uses N] [--days N] [--note TEXT]` | Create an invite link |
| `ab-relay-admin invites` / `revoke-invite ID` | List or cancel invites |
| `ab-relay-admin users` | List accounts |
| `ab-relay-admin reset-password EMAIL` | Print a new password. There's no email reset. |

To update, run `install.sh` again with the same options. Your data is kept.

## What your relay can and can't see

It sees which hostname a connection is for, when, from where, and how many bytes. It can't read the traffic, because TLS ends on your Mac.

You are the operator of your DNS and your relay, so you are also a party who could get a certificate for your own names. On a relay someone else runs, that party is them. This is the same trust you give any DNS host, and the Mac's certificate monitor (`bridge doctor`) exists to catch it.

## Back up

Everything worth keeping is in `/var/lib/ab-relay`, `/var/lib/ab-relay-control`, `/var/lib/ab-accounts` and `/etc/ab-relay`. Stop the services before you copy. Restoring an old control database on purpose refuses to start, so a restore can't bring back a Mac you removed: move `/var/lib/ab-relay-control/epoch` only if you accept that.

## When something's wrong

| You see | It means | Do this |
|---|---|---|
| `This relay does not list this Mac` | The key isn't on the relay | `ab-relay-admin add-key KEY`, with the key the message shows |
| `certificate request failed` in `journalctl -u ab-relay` | Let's Encrypt couldn't check your domain | Check step 2 with `dig NS bridge.example.com` and open port 53 |
| A connector says it can't reach the server | Your Mac is off, or Remote is off | `bridge remote status` on the Mac |
| The approval window doesn't appear | Apple Bridge isn't running on the Mac | Open Apple Bridge, or run `bridge connector pending` |
