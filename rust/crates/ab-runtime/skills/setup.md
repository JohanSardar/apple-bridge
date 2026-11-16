# Apple Bridge: setup

Everything here runs on the owner's Mac. The owner only clicks Allow, or uses Touch ID, when macOS or Apple Bridge asks.

## Rules

- Do every step yourself. Never give the owner commands, links, codes or steps to follow.
- While you wait on the Mac, say only "Click Allow when your Mac asks."
- Never click Allow, type a password or approve a request for the owner.

## Steps

```sh
bridge up                        # starts Apple Bridge, asks nothing
bridge permissions guide         # opens Settings with a small guide; wait for it to close
bridge link claude               # or codex, cursor, gemini, vscode, zed, goose, opencode, claude-desktop
bridge tailscale --keep-awake --yes   # only for cloud agents or "even when my Mac is shut"
bridge stats --wait              # counts per source as each one finishes
```

`bridge link` adds `bridge mcp` to the agent's settings. The first time the agent starts it, the Mac shows a small Allow panel naming the agent; after the owner clicks Allow it just works. If your own MCP client isn't listed, `bridge link other` prints the config to add. Restart the agent after linking so it picks up the new server. Agents that aren't on the Mac, such as Grok Bot or an agent on a server, can't use this; they use the Tailscale route below, or start `bridge mcp` on the Mac over SSH.

`bridge tailscale` installs and signs in to Tailscale, and `--keep-awake` asks macOS once for the owner's password or Touch ID so the Mac stays awake with the lid closed. It prints one sentence for agents that aren't on the Mac: "Connect to Apple Bridge at `https://<mac>:8443/connect` (or 9443, 4443) and use it from now on." Such an agent asks there, gets a key once the owner clicks Allow, and needs no browser. If the owner's Tailscale rules block a device, `bridge tailscale` names it and prints the rule to add; tell the owner, and never change their rules yourself.

## Checks

- `bridge up --json` prints the address and whether the Helper was found.
- `bridge permissions status --json` shows permissions and never prompts.
- Photos is an optional guide step. Skipping or declining it leaves other sources working. `bridge read photos` queries PhotoKit live; Photos has no synced records or library count in `bridge stats`.
- `bridge doctor` checks the data folder.
- `claude mcp list` or `codex mcp list` shows the agent's view of the connection. `bridge connectors list` shows an Allow request that is still waiting.

## Undo

`bridge unlink claude`, `bridge tailscale --off --keep-awake`, `bridge agents list`, `bridge agents revoke <id> --yes` (only after the owner agrees), `bridge down`.
