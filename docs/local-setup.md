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

## The two levels

The only network difference is whether the app runs on your Mac.

| Level | Who connects | How | Needs |
|---|---|---|---|
| 1, the default | Anything on this Mac: Claude Code, Codex, other apps and bots | `bridge link claude` or `bridge link codex`. For anything else, `bridge link other` prints the details | Nothing else |
| 2, opt-in | Other computers and cloud apps such as ChatGPT | `bridge remote on` | An invite for the hosted relay, or [your own server](self-host.md) |

Level 1 never starts the relay. Level 2 only starts when you run `bridge remote on`, and `bridge remote off` stops it.

## Start and stop

`bridge up` starts the gateway in the background and returns as soon as it is ready, with the address, current counts and Next steps last. It never waits for the first sync; use `bridge stats --watch` to follow that separately. The older `--no-watch` flag is accepted as a no-op. If it is already running, it says so. `bridge down` stops a gateway that `bridge up` started. It leaves any other gateway alone.

Useful options: `--home DIR` picks another data folder, `--api-addr 127.0.0.1:PORT` picks another port, and `--json` prints a result for scripts.

`bridge up` keeps running after you close the terminal but does not start again after a restart. The Apple Bridge app handles starting at login.

Run `bridge down` before updating or uninstalling. Moving, replacing or deleting the installation does not stop a running gateway. If the app owns the gateway, stop it through the app instead. If you already removed the command while it was running, reinstall it, then run `bridge down` before uninstalling again.

## Connect an agent

`bridge link claude` adds the server to Claude Code, then signs in through your browser. The page shows a four-digit code. Check that the Apple Bridge window on your Mac shows the same code, and click Allow. Without the app, `bridge link` prints the command to run in another terminal:

```sh
bridge connector allow --request <id> --code <code>
```

| Option | What it does |
|---|---|
| `--scope project` | Claude Code only: save it in this project's `.mcp.json`. Claude Code then asks you to approve it once: run `claude` in that folder, choose "Use this MCP server", and run `bridge link` again. The default, `user`, works in every project with no extra step. On 2026-11-05 a script drove this two-step path through sign-in on a Linux debug build of the current code with Claude Code 2.1.318, ending at `Connected`. It did not go on to a read by Claude Code, and it has not been repeated on a Mac |
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

`bridge permissions guide` lists Full Disk Access, Contacts, Calendar and Reminders with their status, then walks through any that are missing. For each one it waits for you, then asks macOS for that permission alone or opens the right page in System Settings. You can skip any of them, and the other sources keep working. When it has asked macOS for something, it asks the gateway to restart its connected Helper so a new permission starts reading right away, and says so if it cannot confirm that. An older gateway that cannot restart the Helper needs `bridge down`, then `bridge up`.

The Settings step waits up to ten minutes. If the window is not visible, click System Settings in the Dock or use Mission Control to find the Space containing it; Apple Bridge cannot force a Settings window onto your current Space. Press Ctrl-C at any time and rerun `bridge permissions guide` to continue: existing grants are kept, including after a timeout.

macOS may list Apple Bridge in Accessibility with its switch off without showing a permission prompt. The Notes folder-rename capability checks `AXIsProcessTrusted` through the Helper, which is not a request to grant access. This check stays so Notes folder UI remains available when you have allowed it. The guide does not request Accessibility or switch it on. Legacy `onboard` explicitly requests Accessibility; it is not run by this guide.

Reminders can only be read with Calendar allowed too. Without Calendar, Reminders shows as needing a permission.

`bridge permissions status` only shows the list and never asks macOS for anything. It reads the status from what each source last reported, so it shows "not known yet" until the Helper has connected. JSON uses `granted`, `needs_permission` or `unknown`: needing permission does not prove that you denied a request. Without a terminal, the guide does the same and changes nothing, unless you pass `--yes`.

Mail reports missing local data when its folder can be listed but the supported index is absent. Check that Mail has an account on this Mac and has downloaded its mail. If the folder cannot be listed, Apple Bridge cannot tell an absent account from missing Full Disk Access and keeps the permission advice.

`bridge permissions request <name>` asks for one permission, such as `contacts` or `camera`, and nothing else.

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

These print your real data to the terminal. They need the gateway running, and they only read: each one uses a read-only credential that is removed when it finishes. Add `--json` for structured output; `bridge read --limit N` limits the returned items in either format (up to 200), while metadata such as total counts still describes the full query. See `bridge call --list` for every read operation. `bridge call helper.version` needs no arguments.

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

A real first sync has not been timed: it needs the Helper reading a Mac's real Apple data, and that was out of bounds for testing. Two measurements bound it, both from one run on an 8 GB MacBook Air that was also running other work:

| What was timed | Records | Time |
|---|---|---|
| Real data: a fresh replica home syncing from a clone of a test copy, over the local network path | 195,243 real records, 944 MB stored | 59.5 s |
| Capture: synthetic messages through the real capture path into a fresh home | 195,243 synthetic records, 576 MB stored | 44.5 s |

Neither includes the time to read Apple's own stores, or attachments. So expect minutes, not hours, for a Mac of this size, and treat that as an estimate until a real run confirms it. `bridge stats --watch` shows the real progress. A replica's snapshot appears all at once, so its count stays at zero until the snapshot is complete.

## What has been proven

Measured with [`packaging/dev/local-setup-proof.py`](../packaging/dev/local-setup-proof.py) on an 8 GB MacBook Air, with a release build and Claude Code 2.1.318, running the default `bridge link claude`.

| Step | Fresh, empty home | Fresh home, test copy of data (195,243 records) |
|---|---|---|
| `bridge up` | 0.6 s | 0.3 s |
| `bridge stats` | 0.6 s | 1.5 s |
| Claude Code signed in through `bridge link claude` | 3.6 s | 4.4 s |
| First real read by Claude Code | 8.5 s | 10.3 s |
| Claude Code ran `bridge stats` and read the total | 13.7 s | 14.2 s |

Times are cumulative from the start of `bridge up`, from one run each. On the test copy, `bridge stats` matched the source on all 32 collections, and Claude Code's read through the local MCP route returned the right count. Only counts were checked.

Limits of this proof:

- A script played the browser, using the same requests as the sign-in page, and allowed the request with `bridge connector allow`. So a person's time to read and approve is not included, and the Apple Bridge approval window was not exercised.
- The data home was a clone of `state.db` and `ledger.db` from a frozen test copy, with no Helper running. Live data was not used.
- Claude Code ran under a throwaway `HOME`, with only the login Keychain linked in, so its real configuration was not written. A person's first run also passes Claude Code's own onboarding.
- Codex: `bridge link codex` completed real sign-in in about 2 s under a throwaway `CODEX_HOME`, and `codex mcp list` showed it signed in. No real read was made through Codex.
- `bridge permissions guide` was tested through a fixture only, because a real run would show real macOS prompts. Its Helper call has not run against a real Helper.
- A gateway started by `bridge up` from inside the packaged app has not been tested.
