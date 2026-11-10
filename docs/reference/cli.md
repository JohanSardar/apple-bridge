# bridge command reference

Generated from the CLI's own metadata by `bridge help`. Every command below also accepts the global options listed under `bridge`.

## bridge

```text
bridge <command> [options]
Connect local agents such as Claude Code and Codex to your Mac's Apple data

Getting started:
  up           Start Apple Bridge on this Mac for local agents
  link         Connect Claude Code or Codex to this Mac
  unlink       Disconnect Claude Code or Codex
  permissions  Check or allow the Mac permissions Apple Bridge reads with
  stats        Show how much each source has synced, as counts only
  down         Stop a gateway that bridge up started

Reading data:
  read    Read synced data in the terminal
  search  Search synced data
  call    Call a read operation and print its raw JSON
  skills  Print the agent instructions built into this binary

Access:
  agents      Create, inspect, update or revoke agents
  tokens      Create, update or revoke scoped API tokens
  connectors  Allow or deny apps waiting to connect
  web         Manage browser sign-in for this Mac's local pages

Remote and replicas:
  remote  Use Apple Bridge from other computers; off until you enable it
  pair    Create a pairing code for a replica or iPhone
  join    Join a gateway as a replica with a pairing code

Maintenance:
  status      Show this home's role and sync position
  doctor      Check this home's data, files and credentials
  backup      Copy this home's data to a file
  restore     Replace this home's data with a backup
  selftest    Run a self-test that sends real messages to yourself
  shortcuts   Install the Shortcuts pack
  completion  Print a shell completion script

Global options:
  --accessible       Plain text for screen readers: no color or symbols, named steps, numbered
                     choices (also APPLE_BRIDGE_ACCESSIBLE=1)
  --cwd <DIR>        Run as if started in this folder; relative paths resolve against it
  -d, --debug        Print redacted diagnostic detail on stderr
  --fields <FIELDS>  With --json: keep only these comma-separated fields
  -h, --help         Show help for the command
  --home <DIR>       Data folder (default: ~/Library/Application Support/AppleBridge; env:
                     APPLE_BRIDGE_HOME)
  --json             Print the result as JSON; never prompt
  --no-color         Disable color (also NO_COLOR)
  --non-interactive  Never prompt; fail with the input that is needed instead
  -v, --version      Print the version
  -y, --yes          Confirm destructive changes without asking

Examples:
  Start Apple Bridge on this Mac
    bridge up

  Then connect your agent
    bridge link claude
```

## bridge up

```text
bridge up [options]
Start Apple Bridge on this Mac for local agents

Options:
  --api-addr <ADDRESS>   Local API address, loopback only (default: 127.0.0.1:7780)
  --helper-app <PATH>    Helper app (default: the app this binary ships in)
  --peer-addr <ADDRESS>  Replica feed address (default: 127.0.0.1:7443, or a free port if it's
                         taken)

Global options: see bridge --help

Examples:
  Start Apple Bridge on this Mac
    bridge up

  Start it from a script and read the MCP address
    bridge up --json

Notes:
  Returns once the gateway is ready; syncing continues in the background. Running it again reuses
  the running gateway.
  Piped, stdout holds only the MCP address; the summary goes to stderr.
```

## bridge link

```text
bridge link <CLIENT> [options]
Connect Claude Code or Codex to this Mac

Arguments:
  <CLIENT>  Which agent to connect (accepted: claude, codex, other)

Options:
  --name <NAME>         Name shown in the agent (default: apple-bridge)
  --no-login            Register only; sign in later from the agent
  --no-open             Print the sign-in address instead of opening a browser
  --print               Print the agent's commands instead of running them
  --scope <SCOPE>       Claude Code only: where Claude Code saves the registration. This isn't
                        access; you grant access when you sign in (accepted: user, project, local)
                        (default: user)
  --timeout <DURATION>  Stop waiting for sign-in after this long (default: 10m)

Global options: see bridge --help

Examples:
  Connect Claude Code
    bridge link claude

  Print the commands to run yourself
    bridge link codex --print

Notes:
  Opens a browser for sign-in only on a local, interactive terminal; the address is always printed.
  Piped, stdout holds only the server name; bridge link other prints only the MCP address.
```

## bridge unlink

```text
bridge unlink <CLIENT> [options]
Disconnect Claude Code or Codex

Arguments:
  <CLIENT>  Which agent to disconnect (accepted: claude, codex, other)

Options:
  --name <NAME>         Name shown in the agent (default: apple-bridge)
  --scope <SCOPE>       Claude Code only: where the registration is saved (accepted: user, project,
                        local) (default: user)
  --timeout <DURATION>  Stop waiting for the agent's commands after this long (default: 1m)

Global options: see bridge --help

Examples:
  Disconnect Claude Code
    bridge unlink claude

Notes:
  This removes the agent's registration. To end its access on this Mac too, revoke it with bridge
  agents revoke.
  Piped, stdout holds only the removed server name.
```

## bridge permissions

```text
bridge permissions <command> [options]
Check or allow the Mac permissions Apple Bridge reads with

Commands:
  guide    Walk through the Mac permissions Apple Bridge reads with
  status   Show which permissions are allowed; never asks macOS
  request  Ask macOS for one permission through the Helper

Global options: see bridge --help
```

## bridge permissions guide

```text
bridge permissions guide [options]
Walk through the Mac permissions Apple Bridge reads with

Options:
  --helper-app <PATH>  Helper app to use
  --only <NAMES>       Request only these comma-separated permissions, without prompts, such as
                       contacts,calendar

Global options: see bridge --help

Examples:
  Walk through each permission
    bridge permissions guide

  Request two permissions from a script
    bridge permissions guide --only contacts,calendar

Notes:
  Every step is optional, and permissions you've already allowed are kept. Enter at a prompt
  continues; s skips; Ctrl+C or end of input stops before the next step.
  Without a terminal, pass --only with the permissions to request. Without it, the list with each
  status is printed and the command exits 2.
```

## bridge permissions status

```text
bridge permissions status [options]
Show which permissions are allowed; never asks macOS

Global options: see bridge --help

Examples:
  Check which permissions are allowed
    bridge permissions status

Notes:
  With --json, each permission maps to allowed, needs permission or unknown.
```

## bridge permissions request

```text
bridge permissions request <PERMISSION> [options]
Ask macOS for one permission through the Helper

Arguments:
  <PERMISSION>  Permission name, such as camera

Options:
  --helper-app <PATH>  Helper app to use

Global options: see bridge --help

Examples:
  Ask macOS for camera access
    bridge permissions request camera

Notes:
  Names: contacts, calendar, reminders, photos, location, camera, microphone, screen_recording,
  speech_recognition, focus, bluetooth, notifications, automation_mail, automation_safari,
  automation_shortcuts_events.
```

## bridge stats

```text
bridge stats [options]
Show how much each source has synced, as counts only

Options:
  -a, --all             Include sources with nothing synced
  -f, --follow          Keep showing sync events until you stop it or --timeout passes
  --settle <DURATION>   A source counts as finished after this long without new records; 0 means at
                        once (default: 30s)
  --timeout <DURATION>  With --follow or --wait: stop after this long, such as 30s or 5m; 0 checks
                        once (default for --wait: 10m)
  -w, --wait            Show sync events until every source has finished syncing, then exit

Global options: see bridge --help

Examples:
  See what has synced
    bridge stats

  Wait until every source has finished syncing
    bridge stats --wait

  Follow sync events for 5 minutes
    bridge stats --follow --timeout 5m

  Stream sync events as JSON lines for 5 minutes
    bridge stats --follow --timeout 5m --json

Notes:
  Counts only, never content.
  --wait exits 0 once every source has finished syncing. It exits 1 when a source needs attention,
  such as a missing permission, or when --timeout (default 10m) passes first; syncing continues
  either way.
  --follow keeps going until stopped or --timeout; in CI or under an agent it needs --timeout.
  Reaching --timeout exits 0 because watching was the task.
  With --json, --wait and --follow print one JSON event per line; warnings arrive as
  {"type":"warning"} lines.
```

## bridge down

```text
bridge down [options]
Stop a gateway that bridge up started

Global options: see bridge --help

Examples:
  Stop the gateway before updating
    bridge down

Notes:
  Leaves a gateway that something else started, such as the Apple Bridge app, running.
  Run bridge down before updating or uninstalling. Moving or deleting the installation does not stop
  a running gateway.
```

## bridge read

```text
bridge read <KIND> [options]
Read synced data in the terminal

Arguments:
  <KIND>  What to read (accepted: photos, messages, chats, contacts, calendar, reminders, notes,
          mail, voicemail)

Options:
  -a, --all             Fetch every page, up to 10,000 items
  --chat <CHAT>         Messages only: read this chat
  --days <NUMBER>       Calendar only: how many days ahead (default: 7)
  -n, --limit <NUMBER>  How many items to show in total, 1 to 200 (default: 10)
  --next <CURSOR>       Continue from a previous page
  -q, --query <TEXT>    Messages only: find messages containing this text
  --timeout <DURATION>  Stop fetching pages after this long; what was fetched is reported (default:
                        2m)

Global options: see bridge --help

Examples:
  Show your latest chats
    bridge read chats

  Find messages about dinner
    bridge read messages --query dinner --limit 20

  Read the next page
    bridge read mail --next <CURSOR>

Notes:
  Shows newest first. --limit is the total to show (default 10, cap 200); only what remains is
  fetched. Photos is queried live and follows its pages like the rest; everything else is read from
  what has synced, with its freshness on stderr.
  With --json, --limit 0 still makes one request at the gateway's page size and prints its envelope
  without records, as in earlier releases.
  --next continues with the same filters, window and order. Options you leave out come from the
  cursor; ones that differ are refused.
  Piped, rows are tab-separated with a header line, even when nothing matches. With --json, --fields
  picks fields inside each record's data.
  Uses a temporary read-only credential that's revoked when the read ends.
```

## bridge search

```text
bridge search [TEXT] [options]
Search synced data

Arguments:
  <TEXT>  Text to find; optional with --next

Options:
  -a, --all             Fetch every page, up to 10,000 results
  -n, --limit <NUMBER>  How many results to show in total, 1 to 200 (default: 10)
  --next <CURSOR>       Continue from a previous page
  --timeout <DURATION>  Stop fetching pages after this long; what was fetched is reported (default:
                        2m)

Global options: see bridge --help

Examples:
  Search everything that has synced
    bridge search dentist

  Fetch every match as JSON
    bridge search invoice --all --json

Notes:
  --limit is the total to show (default 10, cap 200). --all fetches every page, up to 10,000
  results. --next continues the same search; TEXT can be left out.
  With --json, --limit 0 still prints one page at the gateway's page size, as in earlier releases.
  Piped, rows are tab-separated with a header line, even when nothing matches.
```

## bridge call

```text
bridge call [OPERATION] [options]
Call a read operation and print its raw JSON

Arguments:
  <OPERATION>  Operation name, such as sources.status

Options:
  --data <JSON>       Arguments as JSON; - reads them from stdin (default: {})
  --file <FILE>       Read the arguments from a JSON file; - means stdin
  --list              List read operations, and Photos writes with --write
  --token-env <NAME>  Environment variable holding an existing token to call with; it's never
                      revoked
  --write             Allow Photos organization writes; needs --token-env and a stable query ID

Global options: see bridge --help

Examples:
  List the operations you can call
    bridge call --list

  Check every source's state
    bridge call sources.status

  Pass arguments on stdin
    echo '{"limit": 5}' | bridge call messages.chats --data -

Notes:
  Prints the operation's raw JSON response. Read-only unless --write; other changes go through MCP.
  A write whose result can't be confirmed is reported as unknown_outcome with its operation ID;
  inspect it before trying again. Writes are never retried.
  The result is the operation's own output; see its output schema in bridge call --list --json.
```

## bridge skills

```text
bridge skills <command> [options]
Print the agent instructions built into this binary

Commands:
  list  List the skills in this binary
  get   Print one skill

Global options: see bridge --help
```

## bridge skills list

```text
bridge skills list [options]
List the skills in this binary

Global options: see bridge --help

Examples:
  List the built-in skills
    bridge skills list
```

## bridge skills get

```text
bridge skills get <NAME> [options]
Print one skill

Arguments:
  <NAME>  Skill name, such as core

Global options: see bridge --help

Examples:
  Print the core skill for your agent
    bridge skills get core

Notes:
  Prints the skill's exact Markdown.
```

## bridge agents

```text
bridge agents <command> [options]
Create, inspect, update or revoke agents

Commands:
  list     List agents
  create   Create an agent and print its token once
  inspect  Show one agent
  update   Change an agent's level or access
  revoke   Revoke an agent's token

Global options: see bridge --help
```

## bridge agents list

```text
bridge agents list [options]
List agents

Options:
  -n, --limit <NUMBER>  How many to show, 1 to 200 (default: 50)
  --next <CURSOR>       Continue from a previous page

Global options: see bridge --help

Examples:
  List agents
    bridge agents list

  List agent IDs as JSON
    bridge agents list --json --fields id,name

  See the next page
    bridge agents list --next <CURSOR>

Notes:
  Lists agents in the order they were created, oldest first, 50 at a time (--limit up to 200).
  --next continues; the JSON gives next_cursor, null on the last page.
  Tokens are never shown again after creation.
```

## bridge agents create

```text
bridge agents create [options]
Create an agent and print its token once

Options:
  --expires-in <DURATION>  Stop the token working after this long, such as 1h, 7d or 90d (default:
                           never)
  --level <LEVEL>          What the agent may do (accepted: full, approve, read) (default: full)
  --name <NAME>            Agent name, 1 to 120 bytes
  --scope <ACCESS>         Access grants, comma-separated, such as messages:read,contacts:read
                           (default: reminders:read)

Global options: see bridge --help

Examples:
  Create a read-only agent for a bot that stops working in 30 days
    bridge agents create --name my-bot --level read --scope messages:read,contacts:read --expires-in 30d

  Create one from a script and keep its ID
    bridge agents create --name ci --level read --expires-in 7d --json

Notes:
  The token prints once, alone on stdout. --scope sets the agent's access grants.
  Without --expires-in the token never expires. On a terminal you're asked first; without one it's
  created with a warning.
```

## bridge agents inspect

```text
bridge agents inspect <AGENT> [options]
Show one agent

Arguments:
  <AGENT>  Agent ID, such as agent:01J…

Global options: see bridge --help

Examples:
  Show one agent
    bridge agents inspect <AGENT>
```

## bridge agents update

```text
bridge agents update <AGENT> [options]
Change an agent's level or access

Arguments:
  <AGENT>  Agent ID

Options:
  --level <LEVEL>   New level (accepted: full, approve)
  --scope <ACCESS>  Replace its access grants, comma-separated

Global options: see bridge --help

Examples:
  Make an agent ask before each change
    bridge agents update <AGENT> --level approve

  Replace an agent's access
    bridge agents update <AGENT> --scope messages:read

Notes:
  A read agent can't be raised to full or approve; revoke it and create a new one.
  Raising the level to full or adding sources asks first; without a terminal, pass --yes.
```

## bridge agents revoke

```text
bridge agents revoke <AGENT> [options]
Revoke an agent's token

Arguments:
  <AGENT>  Agent ID, such as agent:01J…

Global options: see bridge --help

Examples:
  Revoke an agent
    bridge agents revoke <AGENT>

  Revoke one from a script
    bridge agents revoke <AGENT> --yes

Notes:
  Asks first; without a terminal, pass --yes. The agent stays in the list, marked revoked.
```

## bridge agents level

```text
bridge agents level <AGENT> <LEVEL> [options]
Change an agent's level
Deprecated: use bridge agents update <AGENT> --level <LEVEL>.

Arguments:
  <AGENT>  Agent ID
  <LEVEL>  New level (accepted: full, approve)

Global options: see bridge --help

Notes:
  Like the old bridge scope, it still runs without --yes outside a terminal, with a warning, until
  the next major version.
```

## bridge tokens

```text
bridge tokens <command> [options]
Create, update or revoke scoped API tokens

Commands:
  create  Create a token and print it once
  revoke  Revoke a token, agent or replica
  update  Replace a token's, agent's or replica's access

Global options: see bridge --help
```

## bridge tokens create

```text
bridge tokens create [options]
Create a token and print it once

Options:
  --expires-in <DURATION>  Stop the token working after this long, such as 1h, 7d or 90d (default:
                           never)
  --remote                 Allow use through remote access
  --scope <ACCESS>         Access grants, comma-separated (default: reminders:read,system:read)

Global options: see bridge --help

Examples:
  Create a read-only token for a script that stops working in 12 hours
    bridge tokens create --scope reminders:read --expires-in 12h

  Store one in a variable
    TOKEN=$(bridge tokens create --scope calendar:read --expires-in 1h)

Notes:
  The token prints once, alone on stdout. With --json it is in secret.
  Without --expires-in the token never expires. On a terminal you're asked first; without one it's
  created with a warning.
```

## bridge tokens revoke

```text
bridge tokens revoke <CREDENTIAL> [options]
Revoke a token, agent or replica

Arguments:
  <CREDENTIAL>  Agent, token or replica ID

Global options: see bridge --help

Examples:
  Revoke a token
    bridge tokens revoke <CREDENTIAL>

Notes:
  Accepts token, agent and replica IDs. Asks first; without a terminal, pass --yes.
```

## bridge tokens update

```text
bridge tokens update <CREDENTIAL> [options]
Replace a token's, agent's or replica's access

Arguments:
  <CREDENTIAL>  Token, agent or replica ID

Options:
  --scope <ACCESS>  Access grants, comma-separated

Global options: see bridge --help

Examples:
  Replace a credential's access
    bridge tokens update <CREDENTIAL> --scope messages:read

Notes:
  Accepts token, agent and replica IDs. Adding sources asks first; without a terminal, pass --yes.
```

## bridge connectors

```text
bridge connectors <command> [options]
Allow or deny apps waiting to connect

Commands:
  list   List apps waiting for approval
  allow  Allow a waiting app after checking its code
  deny   Deny a waiting app

Global options: see bridge --help
```

## bridge connectors list

```text
bridge connectors list [options]
List apps waiting for approval

Options:
  -n, --limit <NUMBER>  How many to show, 1 to 200 (default: 50)
  --next <CURSOR>       Continue from a previous page

Global options: see bridge --help

Examples:
  See which apps are waiting
    bridge connectors list

Notes:
  Lists waiting requests, the one that expires first at the top, 50 at a time (--limit up to 200).
  --next continues.
```

## bridge connectors allow

```text
bridge connectors allow [options]
Allow a waiting app after checking its code

Options:
  --code <CODE>        Four-digit code shown on the app's page (required)
  --request <REQUEST>  Request ID from bridge connectors list (required)

Global options: see bridge --help

Examples:
  Allow an app after checking its code
    bridge connectors allow --request <REQUEST> --code <CODE>

Notes:
  Only allow an app whose code matches the one on its page.
```

## bridge connectors deny

```text
bridge connectors deny [options]
Deny a waiting app

Options:
  --code <CODE>        Four-digit code shown on the app's page (required)
  --request <REQUEST>  Request ID from bridge connectors list (required)

Global options: see bridge --help

Examples:
  Deny an app
    bridge connectors deny --request <REQUEST> --code <CODE>
```

## bridge connectors pending

```text
bridge connectors pending [options]
List apps waiting for approval
Deprecated: use bridge connectors list.

Global options: see bridge --help
```

## bridge web

```text
bridge web <command> [options]
Manage browser sign-in for this Mac's local pages

Commands:
  enroll    Print a link and QR code to add a passkey
  sessions  List or sign out signed-in browsers

Global options: see bridge --help
```

## bridge web enroll

```text
bridge web enroll [options]
Print a link and QR code to add a passkey

Options:
  --host <URL>  Page address: https://HOST, or http://localhost:PORT (default: this Mac's gateway
                address)
  --public      Use the remote access address

Global options: see bridge --help

Examples:
  Add a passkey on this Mac
    bridge web enroll

  Add one through remote access
    bridge web enroll --public

Notes:
  The link is a one-time secret that works for 10 minutes. It uses the port the gateway listens on.
  Piped, stdout holds only the link and no QR code is drawn.
```

## bridge web sessions

```text
bridge web sessions <command> [options]
List or sign out signed-in browsers

Commands:
  list    List signed-in browsers
  revoke  Sign out browsers

Global options: see bridge --help
```

## bridge web sessions list

```text
bridge web sessions list [options]
List signed-in browsers

Options:
  -n, --limit <NUMBER>  How many to show, 1 to 200 (default: 50)
  --next <CURSOR>       Continue from a previous page

Global options: see bridge --help

Examples:
  List signed-in browsers
    bridge web sessions list

Notes:
  Lists browsers in the order they signed in, oldest first, 50 at a time (--limit up to 200). --next
  continues.
```

## bridge web sessions revoke

```text
bridge web sessions revoke [options]
Sign out browsers

Options:
  --all  Sign out every browser

Global options: see bridge --help

Examples:
  Sign out every browser
    bridge web sessions revoke --all

Notes:
  Asks first; without a terminal, pass --yes.
```

## bridge web lock

```text
bridge web lock [options]
Sign out every browser
Deprecated: use bridge web sessions revoke --all.

Global options: see bridge --help
```

## bridge web devices

```text
bridge web devices [options]
List signed-in browsers
Deprecated: use bridge web sessions list.

Global options: see bridge --help
```

## bridge remote

```text
bridge remote <command> [options]
Use Apple Bridge from other computers; off until you enable it

Commands:
  enable        Turn on remote access through the relay or a private route
  disable       Turn off remote access
  status        Show remote access state
  key           Print this Mac's relay key for a self-hosted relay
  certificates  Manage certificates seen for this Mac's remote address

Global options: see bridge --help
```

## bridge remote enable

```text
bridge remote enable [options]
Turn on remote access through the relay or a private route

Options:
  --domain <DOMAIN>     Custom domain on the relay route
  --name <NAME>         Name for this Mac
  --no-open             Print sign-in links instead of opening a browser
  --open                Open sign-in links in the browser even without a terminal, unless over SSH
  --private             Use the private route; same as --via private
  --relay-url <URL>     Your own relay's address
  --timeout <DURATION>  Stop waiting after this long (default: 10m)
  --via <ROUTE>         Route (default: relay) (accepted: relay, private, tailscale-app, both)

Global options: see bridge --help

Examples:
  Turn on remote access through the relay
    bridge remote enable

  Use your own relay
    bridge remote enable --relay-url https://relay.example.com

Notes:
  Waits until remote access is on, up to --timeout (default 10m). Stopping the wait leaves the
  request with the gateway; check it with bridge remote status.
  With --json it doesn't wait for a person: once a sign-in link is needed it prints outcome pending,
  the link in remote.signInUrl and userActionRequired true.
  --no-open never opens a browser; --open opens one even without a terminal, unless over SSH.
```

## bridge remote disable

```text
bridge remote disable [options]
Turn off remote access

Options:
  --name <NAME>         Name for this Mac
  --private             Turn off the private route; same as --via private
  --timeout <DURATION>  Stop waiting after this long (default: 10m)
  --via <ROUTE>         Route to turn off (default: relay) (accepted: relay, private, tailscale-app,
                        both)

Global options: see bridge --help

Examples:
  Turn off remote access
    bridge remote disable
```

## bridge remote status

```text
bridge remote status [options]
Show remote access state

Options:
  --snippets  Print connection snippets for agents

Global options: see bridge --help

Examples:
  Check remote access
    bridge remote status

  Print agent connection snippets
    bridge remote status --snippets

Notes:
  With --json the state is under remote. Piped without --json it prints the state alone, as before.
  --snippets --json gives {"snippets": TEXT}.
```

## bridge remote key

```text
bridge remote key [options]
Print this Mac's relay key for a self-hosted relay

Options:
  --relay-url <URL>  Save this relay address first

Global options: see bridge --help

Examples:
  Print the key for your relay's allowlist
    bridge remote key

Notes:
  Prints only the 43-character key.
```

## bridge remote certificates

```text
bridge remote certificates <command> [options]
Manage certificates seen for this Mac's remote address

Commands:
  accept  Accept a certificate you requested yourself

Global options: see bridge --help
```

## bridge remote certificates accept

```text
bridge remote certificates accept [FINGERPRINT] [options]
Accept a certificate you requested yourself

Arguments:
  <FINGERPRINT>  SHA-256 fingerprint: 64 hexadecimal characters

Global options: see bridge --help

Examples:
  Accept a certificate you requested
    bridge remote certificates accept <FINGERPRINT>
```

## bridge remote on

```text
bridge remote on [options]
Turn on remote access
Deprecated: use bridge remote enable.

Options:
  --domain <DOMAIN>     Custom domain on the relay route
  --name <NAME>         Name for this Mac
  --no-open             Print sign-in links instead of opening a browser
  --open                Open sign-in links in the browser even without a terminal, unless over SSH
  --private             Use the private route; same as --via private
  --relay-url <URL>     Your own relay's address
  --timeout <DURATION>  Stop waiting after this long (default: 10m)
  --via <ROUTE>         Route (default: relay) (accepted: relay, private, tailscale-app, both)

Global options: see bridge --help

Notes:
  Prints the sign-in link and the address on stdout and opens the link in a browser unless
  --no-open, with or without a terminal.
```

## bridge remote off

```text
bridge remote off [options]
Turn off remote access
Deprecated: use bridge remote disable.

Options:
  --name <NAME>         Name for this Mac
  --private             Turn off the private route; same as --via private
  --timeout <DURATION>  Stop waiting after this long (default: 10m)
  --via <ROUTE>         Route to turn off (default: relay) (accepted: relay, private, tailscale-app,
                        both)

Global options: see bridge --help
```

## bridge remote certificate-accept

```text
bridge remote certificate-accept [FINGERPRINT] [options]
Accept a certificate
Deprecated: use bridge remote certificates accept.

Arguments:
  <FINGERPRINT>  SHA-256 fingerprint: 64 hexadecimal characters

Global options: see bridge --help
```

## bridge pair

```text
bridge pair [options]
Create a pairing code for a replica or iPhone

Options:
  --dial-replica <ADDRESS>  Replica address the gateway should dial
  --expires-in <DURATION>   How long the code works (default: 10m)
  --iphone                  Pair an iPhone; needs --via-remote
  --peer-addr <ADDRESS>     Address the replica should dial
  --scope <ACCESS>          Access grants for the replica, comma-separated (default:
                            reminders:read,system:read)
  --via-remote              Pair through remote access

Global options: see bridge --help

Examples:
  Create a code for a replica
    bridge pair --scope messages:read

  Pair an iPhone through remote access
    bridge pair --iphone --via-remote

Notes:
  The code is a secret that works until --expires-in passes. Piped, stdout holds only the code.
```

## bridge join

```text
bridge join <CODE> [options]
Join a gateway as a replica with a pairing code

Arguments:
  <CODE>  Pairing code from bridge pair

Options:
  --listen <ADDRESS>  Listen for the gateway on this address

Global options: see bridge --help

Examples:
  Join a gateway as a replica
    bridge join <CODE>

Notes:
  Piped, stdout holds only the replica's home.
```

## bridge status

```text
bridge status [options]
Show this home's role and sync position

Global options: see bridge --help

Examples:
  Show this home's role
    bridge status
```

## bridge doctor

```text
bridge doctor [options]
Check this home's data, files and credentials

Options:
  --check  Exit 1 with an unhealthy_home error when the report finds problems

Global options: see bridge --help

Examples:
  Check this home
    bridge doctor

  Check it from a script
    bridge doctor --json

Notes:
  A finished report exits 0, even when a check fails. Pass --check to exit 1 with unhealthy_home
  instead.
```

## bridge backup

```text
bridge backup <FILE> [options]
Copy this home's data to a file

Arguments:
  <FILE>  File to write

Options:
  --force  Replace the file if it exists

Global options: see bridge --help

Examples:
  Back up this home
    bridge backup ~/apple-bridge.db

  Replace an older backup from a script
    bridge backup ~/apple-bridge.db --force

Notes:
  Refuses an existing file unless --force, or you confirm at the prompt. The backup is written
  privately and moved into place whole.
```

## bridge restore

```text
bridge restore [FILE] [options]
Replace this home's data with a backup

Arguments:
  <FILE>  Backup file

Options:
  --accept-ledger-loss  Restore even though the command ledger is missing or doesn't match
  --backup <FILE>       Backup file; same as the positional FILE

Global options: see bridge --help

Examples:
  Restore a backup
    bridge restore ~/apple-bridge.db

  Restore from a script after reviewing the effects
    bridge restore ~/apple-bridge.db --yes

Notes:
  Stop the gateway first with bridge down. Shows what will be replaced and asks; without a terminal,
  pass --yes.
```

## bridge selftest

```text
bridge selftest <command> [options]
Run a self-test that sends real messages to yourself

Commands:
  messages  Prove Messages operations by sending real messages to your own chat

Global options: see bridge --help
```

## bridge selftest messages

```text
bridge selftest messages [options]
Prove Messages operations by sending real messages to your own chat

Options:
  --force              Run again even if already proven
  --helper-app <PATH>  Helper app to use
  --no-ui              Skip steps that use the Messages window

Global options: see bridge --help

Examples:
  Prove Messages works on your own chat
    bridge selftest messages

Notes:
  This sends real messages to your own chat through the Helper.
```

## bridge shortcuts

```text
bridge shortcuts <command> [options]
Install the Shortcuts pack

Commands:
  install  Open the Shortcuts pack for import

Global options: see bridge --help
```

## bridge shortcuts install

```text
bridge shortcuts install [options]
Open the Shortcuts pack for import

Options:
  --pack <DIR>  Pack folder (default: .build/shortcuts)

Global options: see bridge --help

Examples:
  Open the Shortcuts pack
    bridge shortcuts install
```

## bridge shortcuts install-pack

```text
bridge shortcuts install-pack [options]
Open the Shortcuts pack
Deprecated: use bridge shortcuts install.

Options:
  --pack <DIR>  Pack folder (default: .build/shortcuts)

Global options: see bridge --help
```

## bridge completion

```text
bridge completion <SHELL> [options]
Print a shell completion script

Arguments:
  <SHELL>  bash, zsh or fish

Global options: see bridge --help

Examples:
  Add completion to zsh
    bridge completion zsh > ~/.zfunc/_bridge
```

## bridge help

```text
bridge help [COMMAND]... [options]
Show help, list every command or search them offline

Arguments:
  <COMMAND>  Command path, or search <TEXT>

Options:
  --all  List every command

Global options: see bridge --help

Examples:
  Search commands offline
    bridge help search token

  List every command as JSON
    bridge help --all --json
```

## bridge start

```text
bridge start [options]
Run the gateway in the foreground; bridge up is usually easier

Options:
  --api-addr <ADDRESS>        Local API address (default: 127.0.0.1:7780)
  --blob-cap <NUMBER>         Attachment storage limit in bytes (default: 0)
  --blob-grace-ms <NUMBER>    Attachment removal grace in milliseconds (default: 0)
  --helper-app <PATH>         Helper app
  --listen <ADDRESS>          Replica listen address
  --peer-addr <ADDRESS>       Replica feed address
  --retention-age <DURATION>  Journal age to keep (default: 0)
  --retention-rows <NUMBER>   Journal rows to keep (default: 0)
  --role <ROLE>               What this process runs as (accepted: gateway, replica) (default:
                              gateway)
  --socket <PATH>             Helper socket

Global options: see bridge --help

Examples:
  Run the gateway in the foreground
    bridge start --role gateway --api-addr 127.0.0.1:7780 --peer-addr 127.0.0.1:7443

Notes:
  Used by the Apple Bridge app's background service.
```

## bridge connect

```text
bridge connect <CODE> [options]
Run the private replica client

Arguments:
  <CODE>  Connection code

Options:
  --listen <ADDRESS>  Local address to serve (default: 127.0.0.1:7780)

Global options: see bridge --help
```

## bridge install-credentials

```text
bridge install-credentials [options]
Prepare credentials for the background service

Global options: see bridge --help

Notes:
  Used by the Apple Bridge app.
```

## bridge revoke

```text
bridge revoke <CREDENTIAL> [options]
Revoke an agent or token
Deprecated: use bridge tokens revoke.

Arguments:
  <CREDENTIAL>  Agent, token or replica ID

Global options: see bridge --help
```

## bridge scope

```text
bridge scope <CREDENTIAL> [options]
Change a token's access
Deprecated: use bridge tokens update.

Arguments:
  <CREDENTIAL>  Agent, token or replica ID

Options:
  --scope <ACCESS>  Access grants, comma-separated (default: reminders:read,system:read)

Global options: see bridge --help
```

## bridge setup

```text
bridge setup [options]
Run the Messages self-test, which sends real messages to yourself
Deprecated: use bridge selftest messages.

Options:
  --force              Run again even if already proven
  --helper-app <PATH>  Helper app to use
  --no-ui              Skip steps that use the Messages window

Global options: see bridge --help

Notes:
  This sends real messages to your own chat through the Helper.
```
