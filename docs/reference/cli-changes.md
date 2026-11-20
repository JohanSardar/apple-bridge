# CLI changes in this release

Use the current command and flag spellings below when updating scripts. The full [CLI reference](cli.md) and [machine-readable surface](cli-surface.json) list options by command.

## What a script should do now

| If a script… | Do this |
|---|---|
| Parses JSON from `status`, `doctor`, `agents list`, `agents create`, `agents update`, `web sessions list`, `connectors list` or `connectors allow` | Add `--json`. Piped output for some older commands still defaults to JSON. Set `APPLE_BRIDGE_LEGACY_JSON=0` to request plain output. |
| Reads the human summary of `bridge up` or `bridge down` from stdout | Read stderr. Piped `bridge up` prints only the MCP address on stdout and piped `bridge down` prints nothing there; `--json` is unchanged. Judge `down` by its exit code |
| Reads the result of `bridge link`, `unlink` or `join` piped | Piped, stdout holds only the result: the server name for `link` and `unlink`, the JSON config for `link other` (earlier releases printed the MCP address), and the replica's home for `join`. The summary goes to stderr |
| Reads `bridge web enroll` piped | Stdout holds only the link, which uses the port the gateway actually listens on. No QR code is drawn when piped |
| Reads the token from `bridge agent create` | Pass `--json` and read `.token`. In a terminal the token alone goes to stdout and the receipt to stderr. |
| Revokes from a script without `--yes` | Use `agents revoke`, `tokens revoke` or `web sessions revoke --all` with `--yes`. |
| Widens an agent's access from a script | `agents update --level full`, and a `--scope` that adds sources, show the agent, its old and new access and what changes, then ask. Without a terminal they need `--yes`. Narrowing access needs no confirmation |
| Runs `bridge permissions guide` without a terminal | Use `bridge permissions status --json` for automation. The guide and individual requests need a local terminal on the Mac. Headless runs and `--non-interactive` exit 2 with `terminal_required` and `data.outcome: pending` in JSON. |
| Treats `doctor` exit 0 as healthy | Read `ok` in `doctor --json` or use `doctor --check --json`. An unhealthy check exits 1 with a single error document on stderr and the report in `data.report`. |
| Waits for the first sync with `stats --follow` | Use `stats --wait`. It exits 1 on timeout or if a source needs attention. The JSON `all_finished` event gives the outcome `finished`, `partial` or `failed`. `stats --follow` observes changes until interrupted or the timeout passes. `stats --watch` has been removed. |
| Parses `call --list` or `skills list` piped | Piped output is tab-separated with a header line. `call --list` has `Operation`, `Scope` and the full `Description`; `--json` gives every field |
| Parses `read` or `search` piped | Rows are tab-separated with a header line, also when nothing matches: `When`, `Title`, `ID` for `search` and each source's own columns for `read`. Long text is no longer shortened when piped. Notes such as "No results" go to stderr |
| Parses `agents list`, `connectors list` or `web sessions list` with `--json` | The plural spellings key the result by noun. For example, `agents list --json` gives `agents` and `next_cursor`. The singular aliases `agent`, `connector` and `token` are still accepted for supported subcommands. |
| Reads JSON from `tokens create` | `tokens create --json` gives `{"token":{"id","scopes","remote","expiresAt"},"secret":…,"outcome":"created","next":[…]}`. `token create --json` keeps the old flat shape, and piped `tokens create` and `token create` still print the bare token |
| Checks for exit code 1 on bad input | Usage errors and missing input exit 2. Operational failures exit 1. Ctrl+C and declining a prompt exit 130. `start` stopped by SIGTERM shuts down quietly and exits 143. |
| Lists agents, connectors or browser sessions in a home that doesn't exist | It's an error now, as it was in earlier releases: exit 1 with `not_set_up`, `Apple Bridge isn't set up in <home>.` and `bridge up --home <home>`. An existing, empty home still lists nothing and exits 0 |
| Reads `read … --limit 0 --json` or `search … --limit 0 --json` | Use a limit of at least 1 in every output mode. |
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
- Standard boolean switches take no value. `--non-interactive=true` and `--non-interactive=false` are accepted.
- An option that takes one value says `--name can be given once.` when repeated, and exits 2.
- Unknown commands and options exit 2. Use two dashes for long options. Each command accepts only members of its option list.
- Nothing after `--` is rewritten.
- Errors, `--debug` lines, receipts and suggested commands never show credentials from an address; they appear as `***`. For the same reason `agents create --name` refuses a name that holds a user name and password, such as `https://user:pass@host`; this is a deliberate safety correction.
- `--timeout` takes a duration such as `30s`, `5m` or `2h`; a bare number still means seconds.
- `--limit` must be at least 1. Above 200 it is clamped with a `limit_clamped` warning.
- Defaults in help read `(default: X)`.
- `read calendar --days` takes any number of days from 1 up, as before.
- Enumerated values, such as `--level`, are listed in `--help --json` and offered by shell completion. List commands also answer to `ls`.

## Renamed commands

| Current command | Removed command |
|---|---|
| `bridge agents update <ID> --level <LEVEL>` | `bridge agents level` |
| `bridge tokens revoke`, `bridge tokens update` | `bridge revoke`, `bridge scope` |
| `bridge connectors list` | `bridge connectors pending` |
| `bridge web sessions list`, `bridge web sessions revoke --all` | `bridge web devices`, `bridge web lock` |
| `bridge shortcuts install` | `bridge shortcuts install-pack` |

`start` and `install-credentials` remain hidden runtime entry points. The legacy `setup` entry uses the same explicit consent as `selftest messages`.

## New

- `bridge backup FILE` refuses an existing file unless `--force`; in a terminal it shows the file and asks. `--yes` never replaces a file. The copy is staged privately and moved into place whole.
- `bridge restore FILE` shows the home, the backup, what it replaces and the ledger check, then asks. Without a terminal it needs `--yes`.
- In a local terminal, `bridge permissions guide --only NAMES` requests just those permissions. End of input or Ctrl+C at a terminal prompt stops before the next request.
- `link <client> --help` lists the options for that client. Claude Code linking, Goose or VS Code install links, Grok Bot linking and Tailscale setup require a local terminal for interactive steps. Use `--print` to preview a link without applying it.
- `bridge read --next CURSOR` and `--all`; `bridge search` takes the same. `read photos` now follows the Photos cursor like the other sources: `--limit` is the total, an empty page that carries a cursor is followed, `--timeout` bounds the whole read, and the JSON keeps the `result_json` envelope with a top-level `next_cursor`.
- `agents list`, `connectors list` and `web sessions list` show 50 at a time (`--limit` up to 200) and give `--next` to continue. Agents are listed oldest first, browser sessions in the order they signed in, and waiting connectors with the one that expires first at the top. The old spellings and piped output without `--json` still list everything unless you pass `--limit` or `--next`.
- `bridge call --data`, `--file FILE` and `--data -` for stdin. `bridge call --describe <OPERATION>` prints the catalog input and output schemas offline.
- `bridge help search <text>`, `bridge help --all [--json]`, `--help --json` on every command, and `bridge completion bash|zsh|fish`.
- Mutations with `--json` include an `outcome`, such as `created`, `updated`, `unchanged`, `revoked`, `replaced`, `restored` or `pending`, and a `next` list of commands. Repeating a change says so, such as `Agent agent:01… is already revoked.`, and exits 0.
- In GitHub Actions, tokens and pairing codes are masked with `::add-mask::`.
- `bridge stats --wait` exits once every source has finished syncing, or fails as described above; `--timeout 0` checks once. Its recovery text names the home and puts each command on its own line. It suggests `bridge down` and `bridge up` only for a gateway that `bridge up` started; for one the app started it says to quit and reopen the app.
- `bridge doctor --check` exits 1 with `unhealthy_home` when a check fails.
- `bridge agents create --expires-in 30d` and `bridge tokens create --expires-in 12h` issue access that expires. Outside an interactive session, credentials that never expire require explicit `--yes`.
- `selftest messages` and `setup` require `--yes` before sending real messages. `backup`, `restore`, `tailscale` and the Messages self-test accept `--dry-run`. An explicit empty backup path is an input error, including with `--accept-ledger-loss`.
- `link claude-app --print` only prints the Claude desktop configuration. `--no-open` and `--no-login` have been removed.
- `bridge link`, `unlink` and `install-credentials` print a JSON document with `--json`. `link --json` gives `outcome` (`added`, `updated`, `already_added` or `opened`), `client`, `name`, the `command` and `args` the agent starts, `where` it was saved and whether the gateway is `running`. `link --print --json` gives the agent's command as an `argv` array, or the `file` and `config` to add, or the install `url`. `--timeout` for CLI-based links and on `unlink` covers every step, including registering, checking, signing out and removing; the child process is stopped and the confirmed state is reported.
- JSON documents are indented on a terminal and one line when piped. `--fields` works for every command with a result schema; for `read` and `search` it picks fields inside each record's `data`, and for a result keyed by noun, such as `tokens create`, inside that object (`--fields id` gives `{"token":{"id":…}}`).
- `bridge help --all --json` and `--help --json` include a `result_schema` for every command that prints JSON, including `link`, `unlink`, `down`, `join`, `pair`, `connectors list`, `web sessions list` and `tokens create`.

## JSON results

- Single results carry warnings in a `"warnings"` array of `{"reason","message"}`. JSON streams from the new spellings (`stats --wait --json`, `stats --follow --json`) carry them as `{"event":"warning","reason","message"}` lines on stdout.
- `data.why` is gone from errors; the reason is in `message` and `next`.
- `permissions request --json` emits one result after permission follow-up work completes.

## Plain output

- Piped, or with `TERM=dumb`, receipts are sentences at column 0, such as `Apple Bridge is stopped.` and `Revoked agent bot (agent:01…).`, with details as `Label: value` lines.
- On a terminal, a label too long for its column goes on its own line with the value below it, and long values wrap.
- `doctor` warnings are sentences with the command to run on its own line.
- Tables show ages such as `8m`; detail views show local time with an age, such as `2026-11-10 14:03 CET (8m ago)`.
- `permissions status` on a terminal is one table with `Permission`, `Status` and `Used for`; the last column wraps.
- Help headings are bold magenta on a terminal, and help text wraps to the terminal width.

## Reads and cursors

- `--limit` is the total across pages; only the remaining count is asked for. `--all` stops at 10,000.
- A cursor from `--next` carries the read's window, filters, order and home. Options you leave out come from it; one that differs is refused with `conflicting_options`, and a cursor from another home with `invalid_value`.
- Reading stops with `pagination_stalled` when the gateway repeats a cursor or returns empty pages that claim more. `--timeout` (default 2 minutes) bounds the whole read.
- When a later page fails, the JSON error includes `rowsFetched` and `pages` for buffered work. Its recovery command uses the last successfully printed cursor. If there is no such cursor, start the same read again from the beginning.
- Transient read failures are retried a few times with backoff, honouring `Retry-After` up to 10 seconds. A 429 is reported as `rate_limited` with `retryAfterSeconds`. Writes are never retried.
- `read messages --chat ID` for a chat that doesn't exist is `not_found`.
- `call` reports a write whose result is uncertain as `unknown_outcome` with `operationId`, what is confirmed, and a safe command to inspect it.

- For a partially applied change, inspect the confirmed effects. For an indeterminate change, check the existing request ID and affected records before further changes.

`bridge link` configures the agent to start `bridge mcp`. An Apple Bridge entry it registered earlier can be replaced on the next link. A different server with the same name is refused. [ADR 0034](../adr/0034-agents-on-this-mac-start-bridge-mcp.md)
