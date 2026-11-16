# Set up Apple Bridge on your own Mac

Agents on your Mac, such as Claude Code and Codex, connect to one local address: `http://127.0.0.1:7780/mcp`. You need no account, server or domain. Apple Bridge sends nothing off your Mac unless you turn on the Tailscale route yourself, though an agent you connect may send what it reads to its own service.

## Install

```sh
curl -fsSL https://eilmer.si/applebridge/install | sh
```

It needs no Node, Homebrew or `sudo`. [The script](install) puts Apple Bridge in `~/.applebridge`, links `bridge` into `~/.local/bin` and adds that folder to your PATH if it's missing. It checks the download's SHA-256 before changing anything.

To update, run the same command, then `bridge up`. It stops the running version first, and your Mac permissions carry over.

To uninstall, disconnect your agents and remove the files:

```sh
bridge unlink claude        # and: bridge unlink codex, for the agents you connected
bridge down
rm -rf ~/.applebridge ~/.local/bin/bridge
```

Your data stays in `~/Library/Application Support/AppleBridge`, including a copy of your private messages and mail and Apple Bridge's credentials. Delete that folder when you no longer want them. Turn off Mac permissions in System Settings under Privacy & Security.

## Quick start

```sh
bridge up                   # start Apple Bridge. It asks nothing
bridge permissions guide    # a small window walks you through the Mac permissions
bridge link claude          # connect Claude Code. Or: codex, cursor, vscode, zed, gemini, goose, opencode, claude-desktop, other
bridge stats --wait         # counts per source, as each one finishes
```

Then ask your agent: "Use Apple Bridge to show my reminder lists. Then run `bridge stats` and tell me how much has synced."

## Ways to connect

Local is the default. The Tailscale route is off until you turn it on with one command.

| Way | Who connects | How | Needs |
|---|---|---|---|
| On this Mac, the default | Agents on this Mac: Claude Code, Codex, Cursor, VS Code, Zed, Gemini CLI, Goose, OpenCode, the Claude app and any other MCP agent | `bridge link <agent>`. For anything else, `bridge link other` prints the config | Nothing else. The Mac asks you to Allow each new agent once |
| Tailscale, for anything not on this Mac | Agents on a server or another computer, and Grok Bot through its Tailscale plugin | `bridge tailscale`, then give the agent the sentence it prints | A Tailscale account; it installs and starts Tailscale on this Mac |
| Plain SSH | An agent on a server that can SSH into this Mac | Its MCP command is `ssh -T you@your-mac ~/.applebridge/bin/bridge mcp` | Remote Login on this Mac and the server's key |

The Tailscale route serves this Mac on your Tailscale network only, never on the public internet. See [connecting other devices](connectors/devices.md). Nothing leaves this Mac until you turn it on, and `bridge tailscale --off` turns it off again.

## Start and stop

`bridge up` starts the gateway in the background and returns as soon as it is ready, with the address, current counts and next steps last. When its output goes to a pipe, stdout carries only the MCP address and the rest goes to stderr. It never waits for the first sync; use `bridge stats --wait` to follow that separately. The older `--no-watch` flag is accepted as a no-op. If it is already running, it says so. `bridge down` stops a gateway that `bridge up` started. It leaves any other gateway alone.

Useful options: `--home DIR` picks another data folder, `--api-addr 127.0.0.1:PORT` picks another port, and `--json` prints a result for scripts.

`bridge up` keeps running after you close the terminal but does not start again after a restart. The Apple Bridge app handles starting at login.

Run `bridge down` before updating or uninstalling. Moving, replacing or deleting the installation does not stop a running gateway. If the app owns the gateway, stop it through the app instead. If you already removed the command while it was running, reinstall it, then run `bridge down` before uninstalling again.

## Connect an agent

Agents on this Mac start Apple Bridge themselves with one local command, `bridge mcp`. Nothing opens a browser, and nothing needs an account or a network. The first time a given agent connects, the Mac shows the small Apple Bridge panel: "Allow “Cursor” to use your Mac?". Click Allow once; after that the agent just works, including after restarts. Don't Allow keeps it out.

| Agent | Run on the Mac | What it does |
|---|---|---|
| Claude Code | `bridge link claude` | `claude mcp add` |
| Codex | `bridge link codex` | `codex mcp add` |
| Gemini CLI | `bridge link gemini` | `gemini mcp add` |
| Cursor | `bridge link cursor` | Adds it to `~/.cursor/mcp.json` |
| Visual Studio Code | `bridge link vscode` | `code --add-mcp`, or opens VS Code's install link |
| Zed | `bridge link zed` | Adds it to `~/.config/zed/settings.json` |
| OpenCode | `bridge link opencode` | Adds it to `~/.config/opencode/opencode.json` |
| Goose | `bridge link goose` | Opens Goose's install link |
| The Claude app | `bridge link claude-desktop` | Adds it to the Claude app's config. Quit and reopen it |
| Anything else | `bridge link other` | Prints the config to paste |

Every other MCP agent on this Mac uses the same setup, which `bridge link other` prints:

```json
{"mcpServers": {"apple-bridge": {"command": "/Users/you/.applebridge/bin/bridge", "args": ["mcp"]}}}
```

`bridge link` only edits a config file it can read as plain JSON; comment lines at the top are kept. For a file with comments elsewhere it changes nothing and prints the lines to add. Running it again is safe: it says "Already added", and it replaces an older Apple Bridge entry with the same name, such as the address `http://127.0.0.1:7780/mcp` that earlier versions registered.

| Option | What it does |
|---|---|
| `--scope project` | Claude Code only: save it in this project's `.mcp.json`. Claude Code asks you to approve project servers the first time it starts there. The default, `user`, works in every project |
| `--name NAME` | Use another name in the agent |
| `--print` | Show the agent's command, config or install link and change nothing |
| `--timeout DURATION` | Stop waiting for the agent's own `mcp add` command after this long (default `30s`) |

To undo it: `bridge unlink claude`, or the name of another agent whose config it edits. To end that agent's access on this Mac as well, run `bridge agents list`, then `bridge agents revoke <id>`. A revoked agent sees the Allow panel again the next time it starts.

When you allow an agent you choose nothing else: it can read and change Apple data, and its changes happen right away, with no further approval. That includes screenshots, recordings, camera photos, current location, Focus and Photos changes. To have an agent ask you before each change instead, run `bridge agents list`, then `bridge agents update <id> --level approve`; `--level full` switches it back. See [connector approvals](connectors/approvals.md).

`bridge mcp` remembers each allowed agent by the name the agent reports and the app it runs in, such as "Claude Code in Ghostty". The key it was given stays in `local-agents/` in the data folder, readable only by you. If Apple Bridge isn't running, the agent's tools answer "Apple Bridge isn't running on this Mac"; run `bridge up`.

### Grok Bot

Grok Bot can't use Apple Bridge through a custom connector on this Mac. Grok Bot starts a Command connector on Grok's own cloud computer, not on your Mac, and it calls a Remote HTTPS connector from Grok's servers, which can't reach `127.0.0.1`. On 2026-11-16 a Command connector added in Grok Bot on this Mac never started here; the bot reported that its script "doesn't exist on my computer". Grok Bot connects over your Tailscale network instead: connect its Tailscale plugin, run `bridge tailscale` and give your bot the sentence it prints ([use Apple Bridge from another computer](connectors/devices.md)). `bridge link grok-bot` says this and changes nothing.

## Other apps and bots

Agents that speak MCP over HTTP instead of starting a command can use `http://127.0.0.1:7780/mcp` with a token. Create one with the least access it needs:

```sh
bridge agents create --name my-bot --level read --scope messages:read,contacts:read
```

`--level` is `full` by default, so a token with write scopes acts at once. Pass `--level approve` to make every change wait for you.

Send the token as `Authorization: Bearer <token>`, to `/mcp` or to the REST API at `POST /v1/op/<operation>`.

## Mac permissions

`bridge permissions guide` opens a small panel that walks through Contacts, Calendar, Reminders, Photos, Full Disk Access and Screen Recording, skipping any you already allowed. For the first four it asks macOS, and you click Allow in macOS's box. For Full Disk Access and Screen Recording it opens System Settings at the right page, sits beside it, and shows an animation of dragging Apple Bridge into the list; if Apple Bridge is already listed, switch it on. macOS then asks for your password or Touch ID. Cancelling that box doesn't end the guide; drag it in again when you're ready. After the grant, macOS may ask to quit Apple Bridge: click Later, or Quit & Reopen, and the guide carries on either way and starts Apple Bridge again if macOS stopped it. "Not Now" skips a permission, and the panel closes itself when you're done. It needs no terminal, so an agent can start it and you click through.

`--only camera,microphone,location` walks through just those. `--in-terminal` keeps the older walkthrough in the terminal, one prompt at a time, which is what runs without a Helper app. After the guide, Apple Bridge restarts its Helper so new permissions start reading right away.

macOS may list Apple Bridge in Accessibility with its switch off without showing a permission prompt. The Notes folder-rename capability checks `AXIsProcessTrusted` through the Helper, which is not a request to grant access. This check stays so Notes folder UI remains available when you have allowed it. The guide does not request Accessibility or switch it on. Legacy `onboard` explicitly requests Accessibility; it is not run by this guide.

Reminders can only be read with Calendar allowed too. Without Calendar, Reminders shows as needing a permission.

`bridge permissions status` only shows the list and never asks macOS for anything. It reads the status from what each source last reported, so it shows "not known yet" until the Helper has connected. JSON uses `granted`, `needs_permission` or `unknown`: needing permission does not prove that you denied a request. Without a terminal, the guide does the same and changes nothing, unless you pass `--yes`.

Mail reports missing local data when its folder can be listed but the supported index is absent. Check that Mail has an account on this Mac and has downloaded its mail. If the folder cannot be listed, Apple Bridge cannot tell an absent account from missing Full Disk Access and keeps the permission advice.

`bridge permissions request <name>` asks for one permission, such as `contacts` or `camera`, and nothing else. If you click Don't Allow, it reports that the permission was declined; the guide continues with the remaining permissions. To enable it later, open System Settings > Privacy & Security > that source, turn on Apple Bridge, then run `bridge down` and `bridge up`. Repeating the request does not make macOS prompt again after a denial.

## See what has synced

```text
$ bridge stats
✓ Messages        163,101 messages · 708 chats · 6,791 attachments
✓ Contacts        154 contacts
✓ Calendar        1,776 events · 30 calendars
! Notes           nothing yet  (needs a Mac permission; if declined, enable it in System Settings > Privacy & Security > Full Disk Access, then run bridge down and bridge up)
354,321 items in total. Counts only, no content.
```

`bridge stats --wait` prints each source as it starts and finishes, with running counts in between, then stops when all are done. A source counts as finished after 30 seconds without new records (`--settle`), so a source that pauses for longer can be reported early and picks up again in `bridge stats`. Apple Bridge does not know the total in advance, so progress is a count, not a percentage. `--json` prints one line per event, and `--timeout DURATION`, such as `30s` or `5m` (a bare number means seconds), stops watching early. Without a terminal, watching stops after five minutes unless you set `--timeout`. Progress goes to stderr; finished sources and the final line go to stdout. `--watch` is the old spelling and still works. `bridge stats --follow` keeps watching after everything finishes, until you stop it or `--timeout` passes; in CI or under an agent it needs `--timeout`.

## Read from the terminal

```sh
bridge read chats --limit 5
bridge read messages --chat <id>
bridge read messages --query "dinner"
bridge read contacts
bridge read calendar --days 3
bridge read reminders
bridge read notes
bridge read mail
bridge search "invoice"
bridge call sources.status
bridge call records.query --data '{"collection":"reminders.list","limit":5}'
```

These print your real data to the terminal. They need the gateway running, and they only read: each one uses a read-only credential that is removed when it finishes. Add `--json` for structured output; `bridge read --limit NUMBER` limits the returned items in either format (1 to 200; a larger number is clamped with a warning), while metadata such as total counts still describes the full query. When there are more, the CLI prints a `--next` cursor to continue, and `--all` fetches every page up to 10,000 items. A source that has never synced, or that lacks its permission, is an error rather than an empty list. `bridge call` reads its arguments from `--data`, from `--file FILE`, or from stdin with `--data -`; `--args` still works. See `bridge call --list` for every read operation. `bridge call helper.version` needs no arguments.

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

Neither includes the time to read Apple's own stores, or attachments. So expect minutes, not hours, for a Mac of this size, and treat that as an estimate until a real run confirms it. `bridge stats --wait` shows the real progress. A replica's snapshot appears all at once, so its count stays at zero until the snapshot is complete.

## What has been proven

### Agents starting `bridge mcp` (2026-11-16)

On a Mac with macOS 26, a debug build of this change ran against a throw-away home and port with no Helper attached, so no panel could appear on the screen. Requests were allowed with `bridge connectors allow`, the same decision the panel's Allow button makes. Each agent ran headless from a scratch folder, so no real agent configuration changed.

| Agent | First connection | Next connection |
|---|---|---|
| Claude Code (`claude -p`, Haiku) | Asked once as "Claude Code in T3 Code (Nightly)"; allowed 5 s in; answered with its agent id from `apple_status` 12 s after start | No request; same agent id |
| Codex (`codex exec`) | Asked once as "Codex in …"; answered with its agent id 9 s after start | No request; same agent id |
| Cursor (`cursor-agent -p`) | Asked once as "Cursor in …"; answered with its agent id | Not repeated |
| Gemini CLI, OpenCode | `gemini mcp list` and `opencode mcp list` showed the server connected | Their models couldn't run on this account, so no tool call was made |

`bridge mcp` answers `initialize` in about 20 ms and lists the tools at once, before Allow is clicked, so no agent times out while the panel waits. A tool call waits up to 50 s for the click, then tells the agent to try again. After Allow it sends `notifications/tools/list_changed` when the agent's real tool list differs. A revoked agent asked again on its next start. With Apple Bridge stopped, a tool call answered "Apple Bridge isn't running on this Mac" at once.

`bridge link` was run for every agent against a throw-away `HOME`: Claude Code, Codex and Gemini CLI listed the server as connected through their own `mcp get` or `mcp list`; the Cursor, Zed, OpenCode and Claude app files held the entry; an older `http://127.0.0.1:7780/mcp` entry in Claude Code was replaced; and `bridge unlink` removed each one. VS Code and Goose were only printed: neither is installed on that Mac, and opening their links would have launched an app.

### Browser sign-in, before `bridge mcp`

Earlier versions signed agents in through a browser. Measured with [`packaging/dev/local-setup-proof.py`](../packaging/dev/local-setup-proof.py) on an 8 GB MacBook Air, with a release build and Claude Code 2.1.318, running the default `bridge link claude`.

| Step | Fresh, empty home | Fresh home, test copy of data (195,243 records) |
|---|---|---|
| `bridge up` | 0.6 s | 0.3 s |
| `bridge stats` | 0.6 s | 1.5 s |
| Claude Code signed in through `bridge link claude` | 3.6 s | 4.4 s |
| First real read by Claude Code | 8.5 s | 10.3 s |
| Claude Code ran `bridge stats` and read the total | 13.7 s | 14.2 s |

Times are cumulative from the start of `bridge up`, from one run each. On the test copy, `bridge stats` matched the source on all 32 collections, and Claude Code's read through the local MCP route returned the right count. Only counts were checked.

Limits of this proof:

- A script played the browser, using the same requests as the sign-in page, and allowed the request with `bridge connectors allow`. So a person's time to read and approve is not included, and the Apple Bridge approval window was not exercised.
- The data home was a clone of `state.db` and `ledger.db` from a frozen test copy, with no Helper running. Live data was not used.
- Claude Code ran under a throwaway `HOME`, with only the login Keychain linked in, so its real configuration was not written. A person's first run also passes Claude Code's own onboarding.
- Codex: `bridge link codex` completed real sign-in in about 2 s under a throwaway `CODEX_HOME`, and `codex mcp list` showed it signed in. No real read was made through Codex.
- `bridge permissions guide` was tested through a fixture only, because a real run would show real macOS prompts. Its Helper call has not run against a real Helper.
- A gateway started by `bridge up` from inside the packaged app has not been tested.
