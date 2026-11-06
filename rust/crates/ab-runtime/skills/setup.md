# Apple Bridge: setup

Everything here runs on the owner's Mac. It needs no account, server or domain.

## Levels

- **Level 1, anything on this Mac (default):** Claude Code, Codex, other apps and bots, on `http://127.0.0.1:7780/mcp`. `bridge link other` prints what other apps need.
- **Level 2, remote use (optional):** `bridge remote on`. Only when the owner asks for it. Never turn it on for them.

## Steps

```sh
bridge up                  # starts Apple Bridge, asks nothing
bridge link claude         # or: bridge link codex
bridge permissions guide   # run in a terminal; it opens Settings and macOS may ask
bridge stats --watch       # counts per source as each one finishes
```

`bridge link` registers the server, then signs in through the browser. The page shows a four-digit code. The owner allows it in the Apple Bridge window, or with `bridge connector allow --request <id> --code <code>`. Never allow a request on the owner's behalf; they must check the code.

## Checks

- `bridge up --json` prints the address and whether the Helper was found.
- `bridge permissions status --json` shows permissions and never prompts.
- `bridge doctor` checks the data folder.
- `claude mcp list` or `codex mcp list` shows the agent's view of the connection.

## Undo

`bridge unlink claude`, `bridge agent list`, `bridge agent revoke <id>`, `bridge down`.
