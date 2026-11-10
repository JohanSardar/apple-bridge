# CLI changes in this release

The `bridge` command now follows one set of rules for output, errors, prompts and options. Every command, flag and output shape from the previous release still works; this page lists what moved and what to change in scripts. The full surface is in the [CLI reference](cli.md), and a machine-readable copy is in [cli-surface.json](cli-surface.json).

## What a script should do now

| If a script… | Do this |
|---|---|
| Parses JSON from `status`, `doctor`, `agent list`, `agent create`, `agent level`, `web devices`, `connector pending`, `connector allow` or `remote status` | Add `--json`. Piped output stays the old JSON for one more release and prints a notice on stderr; after that, piped output becomes a tab-separated table. Set `APPLE_BRIDGE_LEGACY_JSON=0` to get the new behaviour now |
| Reads the human summary of `bridge up` or `bridge down` from stdout | Read stderr. Piped `bridge up` prints only the MCP address on stdout; `--json` is unchanged |
| Reads the token from `bridge agent create` | Pass `--json` and read `.token`. In a terminal the token alone goes to stdout and the receipt to stderr |
| Revokes from a script without `--yes` | Add `--yes`. For one release the old spellings `agent revoke` and `revoke` still run with a warning; the new `agents revoke` and `tokens revoke` need `--yes`; `restore`, `web lock`, `web sessions revoke` and `backup` over an existing file already need `--yes` (or `--force` for backup) |
| Runs `bridge permissions guide` without a terminal | Name the permissions: `--only contacts,calendar`. Without a terminal and without `--only` or `--yes` it exits 2 and requests nothing |
| Treats `doctor` exit 0 as healthy | `doctor` now exits 1 when its report says `"ok": false`; the report is still on stdout |
| Checks for exit code 1 on bad input | Usage errors and missing input now exit 2; operational failures exit 1; Ctrl+C and a declined prompt exit 130 (SIGTERM 143) |
| Reads `read notes` "Nothing to show" as empty | A source that never synced, lacks its permission or is unavailable is now an error (`source_unavailable` or `permission_missing`). A truly empty, synced source prints a note on stderr and exits 0 |

## Errors

Every failure prints the same layout on stderr:

```text
✗ <what failed and why>

  <benefit>:
    <command>
```

Without colour or a terminal the first line starts with `Error:`. With `--json`, nothing goes to stdout and stderr gets one line:

```json
{"status":"action_required","reason":"not_running","message":"Apple Bridge isn't running from /path.","next":[{"command":"bridge up","argv":["bridge","up"]}]}
```

`status` is `error` or `action_required`. `reason` comes from a fixed list: `invalid_arguments`, `unknown_option`, `unknown_command`, `missing_argument`, `invalid_value`, `conflicting_options`, `invalid_home`, `missing_input`, `confirmation_required`, `terminal_required`, `missing_timeout`, `not_running`, `not_set_up`, `not_found`, `source_unavailable`, `permission_missing`, `permission_denied`, `read_only`, `conflict`, `destination_exists`, `port_in_use`, `gateway_busy`, `not_owned`, `ledger_missing`, `network_error`, `api_error`, `spawn_failed`, `missing_command`, `helper_failed`, `unhealthy_home`, `unproven`, `unknown_outcome`, `timeout`, `invalid_config`, `internal_error`, `canceled`. Commands in `next` keep the `--home` you passed. Text from a remote server appears as `<source> said: …`, never as the headline.

## Options

- Global options work before or after the command: `--home`, `--json`, `--fields`, `--non-interactive`, `-y/--yes`, `--no-color` (and `NO_COLOR`), `-d/--debug`, `-v/--version` (`-V` still works), `-h/--help`.
- `APPLE_BRIDGE_HOME` sets the home; an empty value or a file fails with `invalid_home`.
- Every command refuses options it doesn't declare. For one release, a command that used to ignore an unknown option still runs, prints `! --force is ignored by bridge status and will be refused in the next release.`, and never ignores one that changes safety or the target.
- Nothing after `--` is rewritten.
- Old spellings still work and warn once on stderr: `-home` and other single-dash long options, `--flag=true` and `--flag=false`, `stats --watch` (now `--follow`), `read --text` (now `--query`), `link --no-browser` (now `--no-open`), `call --args` (now `--data`).
- `--timeout` takes a duration such as `30s`, `5m` or `2h`; a bare number still means seconds.
- `--limit` must be 1 or more. Above 200 it is clamped with a `limit_clamped` warning.

## Renamed commands

The old names still work. Those marked *warns* print a one-line deprecation notice on stderr.

| New | Old |
|---|---|
| `bridge agents list\|create\|inspect\|update\|revoke` | `bridge agent …` (silent); `agent level <id> <level>` (warns) |
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
- `bridge read --next CURSOR` and `--all`; `bridge search` takes the same.
- `bridge call --data`, `--file FILE` and `--data -` for stdin.
- `bridge help search <text>`, `bridge help --all [--json]`, `--help --json` on every command, and `bridge completion bash|zsh|fish`.
- Mutations with `--json` include an `outcome`, such as `created`, `updated`, `unchanged`, `revoked`, `replaced` or `restored`. Repeating a change says "Already…" and exits 0.
- In GitHub Actions, tokens and pairing codes are masked with `::add-mask::`.
