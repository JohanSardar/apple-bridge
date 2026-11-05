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

The first start asks Let's Encrypt for a certificate, which needs step 2 to be live. If DNS hasn't spread yet, the service retries every two minutes. Let's Encrypt allows five failed tries an hour, so if it keeps failing, fix DNS and wait up to an hour. To check it:

```sh
ab-relay-admin status
curl https://bridge.example.com/info
```

## 4. Connect your Mac

```sh
bridge remote on --relay-url https://bridge.example.com
```

It prints your address, for example `https://8f3a….bridge.example.com/mcp`. Your Mac has its own certificate for it, so in normal use your relay sees only encrypted traffic. [What your relay can and can't see](#what-your-relay-can-and-cant-see) explains the limit.

## 5. Connect an app

**Claude:** Customize → Connectors → Add → Add custom connector. Paste your address, ending in `/mcp`.

**ChatGPT:** turn on Developer mode in ChatGPT's settings, then create an app with the same address and OAuth. Developer mode needs a paid ChatGPT plan.

A page opens saying which app wants what, with a four-digit code. Your Mac shows the same code in an Allow or Deny window. Check that the codes match, then choose Allow. Your iPhone can approve too, if it's paired. On a Mac with no screen in reach, `bridge connector pending` and `bridge connector allow --request ID --code CODE` do the same.

## Letting other people use your relay

Add accounts, so people can link their own Macs by invite:

```sh
bash install.sh --domain bridge.example.com --accounts
ab-relay-admin invite --note "Sam"
```

Send them the link it prints. They create an account at `app.bridge.example.com`, then run `bridge remote on --relay-url https://bridge.example.com`, and a browser page asks them to confirm their Mac.

An account can link Macs and turn tunnels off. It can't reach anyone's data. Each Mac still approves its own connections. You, as the relay's operator, hold more trust than an account does: see the next section.

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

It sees which hostname a connection is for, when, from where, and how many bytes. In normal use it can't read the traffic, because TLS ends on your Mac.

That protection depends on trusting the relay's operator. The relay answers DNS for your Mac's address, so whoever runs it, or takes it over, can get a valid public certificate for that address without your Mac's key and answer connections in your Mac's place. Connectors trust any valid public certificate, so they would send that party their traffic, including their tokens and the data they read. On your own relay that party is you. On a relay someone else runs, it is them. Nothing in Apple Bridge prevents this.

Your iPhone has one more check. The pairing code carries your Mac's signing key, so a substitute server can't forge commands or approval requests to a phone paired that way.

### The certificate warning

Your Mac looks for this itself. About once an hour while Remote is on, it asks the public Certificate Transparency search at `crt.sh` for certificates covering its address, directly and not through the relay. If one appears that your Mac didn't request, you see a warning in:

- `bridge remote status`, as `ct_warning`
- `bridge doctor`
- the Setup window
- the connector approval window
- `bridge pair --iphone`

This is delayed detection, not prevention:

- The certificate already exists by the time you see the warning, and it may already have been used.
- Public logs can take hours to show a certificate, and the Mac has to be on and online to check.
- The first check treats whatever is already public as yours, because it can't tell old certificates apart. It proves nothing about the time before it ran.
- A certificate that never reaches a public log isn't seen. Connector apps and browsers normally reject those.
- If `crt.sh` can't be reached, `bridge doctor` says so at once. The other places say so after a day.

Each check tells `crt.sh` your Mac's address, which public logs already list.

If you recognise the certificate, for example you requested one for your own domain somewhere else, run `bridge remote certificate-accept FINGERPRINT` with the fingerprint from `bridge doctor`. If you don't, run `bridge remote off` and stop using that relay.

## Back up

Everything worth keeping is in `/var/lib/ab-relay`, `/var/lib/ab-relay-control`, `/var/lib/ab-accounts` and `/etc/ab-relay`. Stop the services before you copy. Restoring an old control database on purpose refuses to start, so a restore can't bring back a Mac you removed: move `/var/lib/ab-relay-control/epoch` only if you accept that.

## When something's wrong

| You see | It means | Do this |
|---|---|---|
| `This relay does not list this Mac` | The key isn't on the relay | `ab-relay-admin add-key KEY`, with the key the message shows |
| `certificate request failed` in `journalctl -u ab-relay` | Let's Encrypt couldn't check your domain | Check step 2 with `dig NS bridge.example.com` and open port 53 |
| A connector says it can't reach the server | Your Mac is off, or Remote is off | `bridge remote status` on the Mac |
| `ct_warning` in `bridge remote status` | A certificate your Mac didn't request is public for its address | See [The certificate warning](#the-certificate-warning) |
| The approval window doesn't appear | Apple Bridge isn't running on the Mac | Open Apple Bridge, or run `bridge connector pending` |

## Moving the sign-in address

Sign-in is at `app.<domain>`, on the relay itself. To serve it somewhere else, change `-app-host` and `AB_PUBLIC_URL` together and restart. The new name must resolve to the relay, and the relay must be able to answer its DNS challenge.
