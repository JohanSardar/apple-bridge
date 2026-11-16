# CLI changes in this release

The `bridge` command now follows one set of rules for output, errors, prompts and options. Every command, flag and output shape from the previous release still works; this page lists what moved and what to change in scripts.

## Compatibility policy

Old spellings and behaviours listed here keep working, with a warning on stderr where noted, until the next major version. Nothing on this page is removed in a minor or patch release. A major version lists every removal on this page before it ships. The exceptions below are deliberate safety corrections and are kept: refusing to replace a file silently, refusing unknown options that change safety or the target, not acting on end of input at a prompt, and requiring `--yes` for the new destructive spellings outside a terminal. The full surface is in the [CLI reference](cli.md), and a machine-readable copy is in [cli-surface.json](cli-surface.json).

## What a script should do now

| If a script… | Do this |
|---|---|
| Parses JSON from `status`, `doctor`, `agent list`, `agent create`, `agent level`, `web devices`, `connector pending`, `connector allow` or `remote status` | Add `--json`. Piped output stays the old JSON until the next major version and prints a notice on stderr; after that, piped output becomes a tab-separated table. Set `APPLE_BRIDGE_LEGACY_JSON=0` to get the new behaviour now |
| Reads the human summary of `bridge up` or `bridge down` from stdout | Read stderr. Piped `bridge up` prints only the MCP address on stdout and piped `bridge down` prints nothing there; `--json` is unchanged. Judge `down` by its exit code |
| Reads the result of `bridge link`, `unlink` or `join` piped | Piped, stdout holds only the result: the server name for `link` and `unlink`, the JSON config for `link other` (earlier releases printed the MCP address), and the replica's home for `join`. The summary goes to stderr |
| Reads `bridge web enroll` piped | Stdout holds only the link, which uses the port the gateway actually listens on. No QR code is drawn when piped |
| Reads the token from `bridge agent create` | Pass `--json` and read `.token`. In a terminal the token alone goes to stdout and the receipt to stderr |
| Revokes from a script without `--yes` | Add `--yes`. Until the next major version the old spellings `agent revoke`, `revoke`, `scope` and `web lock` still run with a warning; the new `agents revoke`, `tokens revoke`, `tokens update` and `web sessions revoke --all` need `--yes`; `restore` and `backup` over an existing file already need `--yes` (or `--force` for backup) |
| Widens an agent's access from a script | `agents update --level full`, and a `--scope` that adds sources, show the agent, its old and new access and what changes, then ask. Without a terminal they need `--yes`. Narrowing access needs no confirmation |
| Runs `bridge permissions guide` without a terminal | Name the permissions: `--only contacts,calendar`. Without a terminal and without `--only` or `--yes` it prints the list with each status on stdout, as before, then exits 2 with `terminal_required` and requests nothing |
| Treats `doctor` exit 0 as healthy | Read `"ok"` in `doctor --json`, or run `doctor --check`, which exits 1 with `unhealthy_home` when a check fails. Plain `doctor` exits 0 whenever the report finishes, as before |
| Waits for the first sync with `stats --follow` | Use `stats --wait`, which exits 0 once every source has finished. It exits 1 with `timeout` when `--timeout` (default 10 minutes) passes first, and 1 with `permission_missing` or `source_unavailable` when a source needs attention; with `--json` the last event is `all_finished` with `outcome` `finished`, `partial` or `failed`. Syncing continues either way. `--follow` follows until it's stopped or `--timeout` passes; in CI or under an agent it needs `--timeout` and otherwise exits 2 with `missing_timeout`. The old `stats --watch` still works exactly as before, including `--settle 0`, `--timeout 0` and exit 0 when it stops watching |
| Parses `call --list` or `skills list` piped | Piped output is tab-separated with a header line. `call --list` has `Operation`, `Scope` and the full `Description`; `--json` gives every field |
| Parses `read` or `search` piped | Rows are tab-separated with a header line, also when nothing matches: `When`, `Title`, `ID` for `search` and each source's own columns for `read`. Long text is no longer shortened when piped. Notes such as "No results" go to stderr |
| Parses `agents list`, `connectors list`, `web sessions list` or `remote status` with `--json` | The new plural spellings key the result by noun: `{"agents":[…],"next_cursor":…}`, `{"connectors":[…]}`, `{"sessions":[…]}`, `{"remote":{…}}`. The old spellings (`agent list`, `connector pending`, `web devices`) and piped output without `--json` keep the old bare shapes until the next major version |
| Reads JSON from `tokens create` | `tokens create --json` gives `{"token":{"id","scopes","remote","expiresAt"},"secret":…,"outcome":"created","next":[…]}`. `token create --json` keeps the old flat shape, and piped `tokens create` and `token create` still print the bare token |
| Checks for exit code 1 on bad input | Usage errors and missing input now exit 2; operational failures exit 1; Ctrl+C and a declined prompt exit 130 (SIGTERM 143). `start` and `connect` stopped by SIGTERM shut down quietly and exit 143 |
| Lists agents, connectors or browser sessions in a home that doesn't exist | It's an error now, as it was in earlier releases: exit 1 with `not_set_up`, `Apple Bridge isn't set up in <home>.` and `bridge up --home <home>`. An existing, empty home still lists nothing and exits 0 |
| Reads `read … --limit 0 --json` or `search … --limit 0 --json` | Still exits 0, as in earlier releases: one request at the gateway's page size, its envelope, no records for `read`, and `next_cursor` null. Without `--json`, `--limit 0` is a usage error |
| Reads `read notes` "Nothing to show" as empty | A source that never synced, lacks its permission or is unavailable is now an error (`source_unavailable` or `permission_missing`). A truly empty, synced source prints a note on stderr and exits 0 |

## Errors

Every failure prints the same layout on stderr:

```text
✗ <what failed and why>

  <benefit>:
    <command>
```

Without colour or a terminal the first line starts with `Error:`. With `--json`, nothing goes to stdout and stderr gets one line. Under `--json` stderr carries only that error document; warnings travel inside the result instead (see [JSON results](#json-results)). The document is the same wherever `--json` sits on the command line.

```json
{"status":"action_required","reason":"not_running","message":"Apple Bridge isn't running from /path.","next":[{"command":"bridge up","argv":["bridge","up"]}]}
```

`status` is `error` or `action_required`. `reason` comes from a fixed list: `invalid_arguments`, `unknown_option`, `unknown_command`, `missing_argument`, `invalid_value`, `conflicting_options`, `invalid_home`, `missing_input`, `confirmation_required`, `terminal_required`, `missing_timeout`, `not_running`, `not_set_up`, `not_found`, `source_unavailable`, `permission_missing`, `permission_denied`, `read_only`, `conflict`, `destination_exists`, `port_in_use`, `gateway_busy`, `not_owned`, `ledger_missing`, `network_error`, `api_error`, `spawn_failed`, `missing_command`, `helper_failed`, `unhealthy_home`, `unproven`, `unknown_outcome`, `timeout`, `invalid_config`, `internal_error`, `canceled`, `rate_limited`, `pagination_stalled`. Commands in `next` keep the `--home` you passed. Text from a remote server appears as `<source> said: …`, never as the headline.

## Options

- Global options work before or after the command: `--home`, `--json`, `--fields`, `--non-interactive`, `-y/--yes`, `--no-color` (and `NO_COLOR`), `-d/--debug`, `-v/--version` (`-V` still works), `-h/--help`, `--cwd` and `--accessible`.
- `--cwd DIR` runs as if started in `DIR`: relative paths such as `--home rel` or a backup file resolve against it. Help and completion never use it, so they stay offline and change nothing.
- `--accessible` (or `APPLE_BRIDGE_ACCESSIBLE=1`) prints plain text for screen readers: no colour or symbols, each long step named as `Step 1 of 2: …`, and choices numbered so you answer with a number.
- `APPLE_BRIDGE_HOME` sets the home; an empty value or a file fails with `invalid_home`.
- Global options also work after the command and its arguments, including `bridge help agents --home X` and `bridge help search revoke --json`.
- Boolean options accept `--flag=true`, `--flag=false` and `--no-flag` without a warning; when one is given more than once, the last one wins.
- An option that takes one value says `--name can be given once.` when repeated, and exits 2.
- `bridge typo --help`, `bridge help typo` and `bridge help agents typo` all fail the same way: exit 2 with `Unknown command typo`.
- Every command refuses options it doesn't declare. Until the next major version, a command that used to ignore an unknown option still runs, prints `! --force is ignored by bridge status and will be refused in the next major version.`, and never ignores one that changes safety or the target.
- Nothing after `--` is rewritten.
- Old spellings still work and warn once on stderr: `-home` and other single-dash long options, `read --text` (now `--query`), `link --no-browser` (now `--no-open`), `call --args` (now `--data`). `stats --watch` is hidden and keeps its old meaning; `--wait` is the new name.
- `remote on`, `remote off` and `remote enable`/`disable` accept `--via system` again.
- Options written between `remote` and its action work again, as in earlier releases: `bridge remote --relay-url URL key` saves URL and prints that relay's key, and `bridge remote --snippets status` prints the snippets.
- `--relay-url` refuses an address with a user name or password (`https://user:pass@…`) before anything is saved. Errors, `--debug` lines, receipts and suggested commands never show credentials from an address; they appear as `***`. For the same reason `agents create --name` refuses a name that holds a user name and password, such as `https://user:pass@host`; this is a deliberate safety correction.
- `--timeout` takes a duration such as `30s`, `5m` or `2h`; a bare number still means seconds.
- `--limit` must be 1 or more, except `read` and `search` with `--json`, where `--limit 0` keeps its old meaning (see above). Above 200 it is clamped with a `limit_clamped` warning.
- Defaults in help read `(default: X)`.
- `read calendar --days` takes any number of days from 1 up, as before.
- Enumerated values, such as `--level` and `--via`, are listed in `--help --json` and offered by shell completion. List commands also answer to `ls`.

## Renamed commands

The old names still work. Those marked *warns* print a one-line deprecation notice on stderr.

| New | Old |
|---|---|
| `bridge agents list\|create\|inspect\|update\|revoke` | `bridge agent …` (silent); `agent level <id> <level>` and `agents level` (warn) |
| `bridge tokens create\|update\|revoke` | `bridge token create` (silent); `bridge revoke`, `bridge scope` (warn) |
| `bridge connectors list\|allow\|deny` | `bridge connector allow\|deny` (silent); `connector pending` (warns) |
| `bridge web sessions list`, `bridge web sessions revoke --all` | `bridge web devices`, `bridge web lock` (warn) |
| `bridge remote enable\|disable\|status\|key` | `bridge remote on\|off` (warn) |
| `bridge remote certificates accept <FINGERPRINT>` | `bridge remote certificate-accept` (warns) |
| `bridge shortcuts install` | `bridge shortcuts install-pack` (warns) |
| `bridge selftest messages` | `bridge setup` (warns) |

`tokens update` and `scope` accept agent, token and replica IDs. `start`, `connect` and `install-credentials` are hidden from help and work as before.

## New

- `bridge backup FILE` refuses an existing file unless `--force`; in a terminal it shows the file and asks. `--yes` never replaces a file. The copy is staged privately and moved into place whole.
- `bridge restore FILE` shows the home, the backup, what it replaces and the ledger check, then asks. Without a terminal it needs `--yes`.
- `bridge permissions guide --only NAMES` requests just those permissions, with no prompts. End of input or Ctrl+C at a prompt stops before the next request.
- `bridge link --timeout` and `bridge remote enable --timeout` (default 10 minutes) stop waiting without undoing anything.
- `bridge read --next CURSOR` and `--all`; `bridge search` takes the same. `read photos` now follows the Photos cursor like the other sources: `--limit` is the total, an empty page that carries a cursor is followed, `--timeout` bounds the whole read, and the JSON keeps the `result_json` envelope with a top-level `next_cursor`.
- `agents list`, `connectors list` and `web sessions list` show 50 at a time (`--limit` up to 200) and give `--next` to continue. Agents are listed oldest first, browser sessions in the order they signed in, and waiting connectors with the one that expires first at the top. The old spellings and piped output without `--json` still list everything unless you pass `--limit` or `--next`.
- `bridge call --data`, `--file FILE` and `--data -` for stdin.
- `bridge help search <text>`, `bridge help --all [--json]`, `--help --json` on every command, and `bridge completion bash|zsh|fish`.
- Mutations with `--json` include an `outcome`, such as `created`, `updated`, `unchanged`, `revoked`, `replaced`, `restored` or `pending`, and a `next` list of commands. Repeating a change says so, such as `Agent agent:01… is already revoked.`, and exits 0.
- In GitHub Actions, tokens and pairing codes are masked with `::add-mask::`.
- `bridge stats --wait` exits once every source has finished syncing, or fails as described above; `--timeout 0` checks once. Its recovery text names the home and puts each command on its own line. It suggests `bridge down` and `bridge up` only for a gateway that `bridge up` started; for one the app started it says to quit and reopen the app.
- `bridge doctor --check` exits 1 with `unhealthy_home` when a check fails.
- `bridge agents create --expires-in 30d` and `bridge tokens create --expires-in 12h` issue access that ends on its own. Without it, access doesn't expire, as before. In a terminal without `--expires-in`, both show the level, scope and expiry and ask `Create agent <name> with access that never expires? (y/N)`; declining creates nothing. Without a terminal they still create it and warn `! <name>'s token never expires. Pass --expires-in to limit it.` (with `--json` the warning is in `warnings`). `--yes` skips the question. An ordinary create prints only the `✓ Created` receipt.
- `bridge remote enable --open` opens the sign-in link in a browser even without a terminal, unless the session is over SSH. Piped, the link alone goes to stdout as soon as it's known, and the address once Remote is on. The Helper's Enable Remote button uses it. `remote on` behaves exactly as before: without a terminal it opens the link and is judged by its exit code.
- `--no-open` never opens a browser, on `remote enable` and `remote on`. `link --no-open` and `link --no-login` are still accepted and do nothing, because `link` no longer signs in or opens a browser.
- `bridge remote enable --json` doesn't wait for a person. Once a sign-in link is needed it prints `{"remote":{"state":"pending","signInUrl":…},"outcome":"pending","userActionRequired":true,"message":…,"next":[…]}` and exits 0; check later with `bridge remote status --json`. `remote on --json` prints only its JSON document.
- A failed remote attempt says `Couldn't reach <host>. <cause>.` and points to `bridge remote status`. `bridge up` shows the cause of a remote error and `See why: bridge remote status`.
- `bridge link`, `unlink` and `install-credentials` print a JSON document with `--json`. `link --json` gives `outcome` (`added`, `updated`, `already_added` or `opened`), `client`, `name`, the `command` and `args` the agent starts, `where` it was saved and whether the gateway is `running`. `link --print --json` gives the agent's command as an `argv` array, or the `file` and `config` to add, or the install `url`. `--timeout` on `link` and `unlink` covers every step, including registering, checking, signing out and removing; the child process is stopped and the confirmed state is reported.
- JSON documents are indented on a terminal and one line when piped. `--fields` works for every command with a result schema; for `read` and `search` it picks fields inside each record's `data`, and for a result keyed by noun, such as `tokens create`, inside that object (`--fields id` gives `{"token":{"id":…}}`).
- `bridge help --all --json` and `--help --json` include a `result_schema` for every command that prints JSON, including `link`, `unlink`, `down`, `join`, `pair`, `connectors list`, `web sessions list`, `remote enable` and `tokens create`.

## JSON results

- Single results carry warnings in a `"warnings"` array of `{"reason","message"}`. JSON streams from the new spellings (`stats --wait --json`, `stats --follow --json`) carry them as `{"type":"warning","reason","message"}` lines on stdout.
- The old spellings whose result is a bare array, such as `web devices --json`, can't hold warnings; they print them as `{"status":"warning",…}` lines on stderr, as before.
- `data.why` is gone from errors; the reason is in `message` and `next`.
- Raw `call` output and the `stats --watch` events are unchanged.

## Plain output

- Piped, or with `TERM=dumb`, receipts are sentences at column 0, such as `Apple Bridge is stopped.` and `Revoked agent bot (agent:01…).`, with details as `Label: value` lines.
- On a terminal, a label too long for its column goes on its own line with the value below it, and long values wrap.
- `remote status` shows no warning mark when Remote is off. `doctor` warnings are sentences with the command to run on its own line.
- Tables show ages such as `8m`; detail views show local time with an age, such as `2026-11-10 14:03 CET (8m ago)`.
- `permissions status` on a terminal is one table with `Permission`, `Status` and `Used for`; the last column wraps.
- Help headings are bold magenta on a terminal, and help text wraps to the terminal width.

## Reads and cursors

- `--limit` is the total across pages; only the remaining count is asked for. `--all` stops at 10,000.
- A cursor from `--next` carries the read's window, filters, order and home. Options you leave out come from it; one that differs is refused with `conflicting_options`, and a cursor from another home with `invalid_value`.
- Reading stops with `pagination_stalled` when the gateway repeats a cursor or returns empty pages that claim more. `--timeout` (default 2 minutes) bounds the whole read.
- When a later page fails, the JSON error includes `rowsFetched`, `pages`, the starting `cursor` and a `resume` cursor.
- Transient read failures are retried a few times with backoff, honouring `Retry-After` up to 10 seconds. A 429 is reported as `rate_limited` with `retryAfterSeconds`. Writes are never retried.
- `read messages --chat ID` for a chat that doesn't exist is `not_found`.
- `call` reports a write whose result is uncertain as `unknown_outcome` with `operationId`, what is confirmed, and a safe command to inspect it.

## Compatibility exceptions

- `agents create` still defaults to `--level full` and access that doesn't expire, because the setup scripts and lab rely on it; outside a terminal it warns instead of asking. `tokens create` still prints the bare token on stdout.
- The old spellings `scope`, `agent level`, `agents level`, `agent revoke`, `revoke` and `web lock` still run without `--yes` outside a terminal, with a warning, until the next major version. `agent level <id> full` and `scope <id> <grants>` behave the same way: the earlier release had no confirmation for either, so both keep running, while `agents update` asks and needs `--yes` without a terminal.
- `read … --limit 0 --json` and `search … --limit 0 --json` exit 0 with one page, as before.
- Piped output without `--json` from `agents list`, `agents create`, `agents update`, `connectors list`, `web sessions list` and `remote status` keeps the old JSON shape until the next major version.
- `bridge link` now has the agent start `bridge mcp` and ask on the Mac once, instead of registering `http://127.0.0.1:7780/mcp` and signing in through a browser ([ADR 0034](../adr/0034-agents-on-this-mac-start-bridge-mcp.md)). An entry it registered earlier is replaced the next time it runs; a different server with the same name is refused. `bridge link claude-desktop` now edits the Claude app's local config; `bridge link other` prints a config instead of an address.
