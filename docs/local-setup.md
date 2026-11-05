# Set up Apple Bridge on your own Mac

Agents on your Mac, such as Claude Code and Codex, connect to one local address: `http://127.0.0.1:7780/mcp`. You need no account, server or domain. Apple Bridge sends nothing off your Mac unless you turn on remote use yourself, though an agent you connect may send what it reads to its own service.

## Quick start

```sh
bridge up                   # start Apple Bridge. It asks nothing
bridge link claude          # connect Claude Code. Or: bridge link codex
bridge permissions guide    # allow the Mac permissions you want
bridge stats --watch        # counts per source, as each one finishes
```

Then ask your agent: "Use Apple Bridge to show my reminder lists. Then run `bridge stats` and tell me how much has synced."

## The three levels

| Level | Who connects | How | Needs |
|---|---|---|---|
| 1, the default | Agents on this Mac | `bridge link claude` or `bridge link codex` | Nothing else |
| 2 | Other apps and bots on this Mac | The same address. `bridge link other` prints the details | Nothing else |
| 3, opt-in | Other computers and cloud apps such as ChatGPT | `bridge remote on` | An invite for the hosted relay, or [your own server](self-host.md) |

Levels 1 and 2 never start the relay. Level 3 only starts when you run `bridge remote on`, and `bridge remote off` stops it.

## Start and stop

`bridge up` starts the gateway in the background and shows the address, what has synced so far and what to do next. If it is already running, it says so. `bridge down` stops a gateway that `bridge up` started. It leaves any other gateway alone.

Useful options: `--home DIR` picks another data folder, `--api-addr 127.0.0.1:PORT` picks another port, and `--json` prints a result for scripts.

`bridge up` keeps running after you close the terminal but does not start again after a restart. The Apple Bridge app handles starting at login.

## Connect an agent

`bridge link claude` adds the server to Claude Code, then signs in through your browser. The page shows a four-digit code. Check that the Apple Bridge window on your Mac shows the same code, and click Allow. Without the app, `bridge link` prints the command to run in another terminal:

```sh
bridge connector allow --request <id> --code <code>
```

| Option | What it does |
|---|---|
| `--scope project` | Claude Code only: save it in this project's `.mcp.json`. Claude Code then asks you to approve it once: run `claude` in that folder, choose "Use this MCP server", and run `bridge link` again. The default, `user`, works in every project with no extra step |
| `--name NAME` | Use another name in the agent |
| `--no-login` | Add the server now and sign in later with `claude mcp login apple-bridge` |
| `--no-browser` | Claude Code only, for a session without a browser: prints the address, then you paste the final address back |
| `--print` | Show the agent's commands and run nothing |

Without a terminal, for example when an agent runs it, `bridge link` adds the server and prints the sign-in command for you to run. It never allows a request itself.

To undo it: `bridge unlink claude`. To end that agent's access on this Mac as well, run `bridge agent list`, then `bridge agent revoke <id>`.

When you allow a request you choose nothing else: the agent can read what it asked for, and every change waits for your approval. See [connector approvals](connectors/approvals.md).

## Other apps and bots

`bridge link other` prints the address. Apps that support MCP sign-in work the same way as Claude Code. For a bot without sign-in, create a token with the least access it needs:

```sh
bridge agent create --name my-bot --level read --scope messages:read,contacts:read
```

Send the token as `Authorization: Bearer <token>`, to `/mcp` or to the REST API at `POST /v1/op/<operation>`.

## Mac permissions

`bridge permissions guide` lists Full Disk Access, Contacts, Calendar and Reminders with their status, then walks through any that are missing. For each one it waits for you, then asks macOS or opens the right page in System Settings. You can skip any of them, and the other sources keep working.

`bridge permissions status` only shows the list and never asks macOS for anything. It reads the status from what each source last reported, so it shows "not known yet" until the Helper has connected. Without a terminal, the guide does the same and changes nothing.

`bridge permissions request <name>` asks for one optional permission such as `camera`.

## See what has synced

```text
$ bridge stats
  ✓ Messages     163,101 messages · 708 chats · 6,791 attachments
  ✓ Contacts     154 contacts
  ✓ Calendar     1,776 events · 30 calendars
  ! Notes        nothing yet  (needs a Mac permission: run bridge permissions guide)
  354,321 items in total. Counts only, no content.
```

`bridge stats --watch` prints each source as it starts and finishes, with running counts in between, then stops when all are done. A source counts as finished after 30 seconds without new records (`--settle`), so a source that pauses for longer can be reported early and picks up again in `bridge stats`. Apple Bridge does not know the total in advance, so progress is a count, not a percentage. `--json` prints one line per event, and `--timeout SECONDS` stops watching early. Without a terminal, watching stops after five minutes unless you set `--timeout`.

## Read from the terminal

```sh
bridge read chats --limit 5
bridge read messages --chat <id>
bridge read messages --text "dinner"
bridge read contacts
bridge read calendar --days 3
bridge read reminders
bridge read notes
bridge read mail
bridge search "invoice"
bridge call sources.status
bridge call records.query --args '{"collection":"reminders.list","limit":5}'
```

These print your real data to the terminal. They need the gateway running, and they only read: each one uses a read-only credential that is removed when it finishes. Add `--json` for the full answer, and see `bridge call --list` for every read operation.

## Instructions for your agent

The binary carries instructions that match its version:

```sh
bridge skills list
bridge skills get core     # how to read safely, and which tools to start with
bridge skills get setup    # how to set up this Mac
```

To teach an agent, add this to its instructions: "Run `bridge skills get core` and follow it."

## Help and errors

`bridge --help`, `bridge <command> --help` and `bridge --version` work everywhere. Errors print what went wrong, then a `next:` line saying what to do. Usage errors exit with status 1.

## Try it on a throw-away folder

To try the commands without touching your normal data, use a separate folder and port:

```sh
export APPLE_BRIDGE_SECRET_STORE=file
bridge up --home /tmp/bridge-try --api-addr 127.0.0.1:7790 --peer-addr 127.0.0.1:0
bridge stats --home /tmp/bridge-try
bridge down --home /tmp/bridge-try
```

`APPLE_BRIDGE_SECRET_STORE=file` keeps this folder's secrets in a file inside it, so it does not touch your Keychain. Use it only for test folders.

## How long the first sync takes

A real first sync has not been timed: it needs the Helper reading a Mac's real Apple data, and that was out of bounds for testing. What has been measured is the gateway's side:

- **Storing records:** 195,243 synthetic messages went through the real capture path into a fresh home in 44.5 s on an 8 GB MacBook Air that was also running other work (4,391 records per second, single run). That is the same number of records as the test copy, but real records are about three times larger once stored (1.9 GB against 576 MB).
- **Not included:** the time to read Apple's own stores, and attachments.

So expect minutes, not hours, for a Mac of this size, and treat that as an estimate until a real run confirms it. `bridge stats --watch` shows the real progress.

## What has been proven

Measured with [`packaging/dev/local-setup-proof.py`](../packaging/dev/local-setup-proof.py) on an 8 GB MacBook Air, with a release build, Claude Code 2.1.318 and `--scope project` in a scratch folder.

| Step | Fresh, empty home | Fresh home, test copy of data (195,243 records) |
|---|---|---|
| `bridge up` | 0.8 s | 0.2 s |
| `bridge stats` | 0.8 s | 1.4 s |
| `bridge link`, server registered | 2.2 s | 2.4 s |
| Signed in | 7.3 s | 7.7 s |
| First real read by Claude Code | 12.8 s | 13.8 s |
| Claude Code ran `bridge stats` and read the total | 17.8 s | 18.6 s |

Times are cumulative from the start of `bridge up`, from one run each. On the test copy, `bridge stats` matched the source on all 32 collections, and Claude Code's read returned the right count through the local MCP route and sign-in. Only counts were checked.

Limits of this proof:

- A script played the browser, using the same requests as the sign-in page, and allowed the request with `bridge connector allow`. So a person's time to read and approve is not included, and the Apple Bridge approval window was not exercised.
- `--scope project` needs one approval in Claude Code. It took about 6 s under script and is left out of the times, because the default scope does not need it. The default `--scope user` flow itself was not run, to keep the real Claude Code configuration unchanged.
- The data home was a clone of `state.db` and `ledger.db` from a frozen test copy, with no Helper running. Live data was not used.
- Codex: `bridge link codex` completed real sign-in in about 7 s and `codex mcp list` showed it signed in. No real read was made through Codex.
- `bridge permissions guide` was tested through a fixture only, because a real run would show real macOS prompts. Its Helper call has not run against a real Helper.
- A gateway started by `bridge up` from inside the packaged app has not been tested.
