# Agents off the Mac use a private network and a key

Amends [ADR 0032](0032-approve-on-the-mac-without-a-code.md) and [ADR 0034](0034-agents-on-this-mac-start-bridge-mcp.md). Apple Bridge is local first. A private network is the only way to use it from anything that is not on the Mac: another computer, a headless server, or Grok Bot's cloud computer.

The network decides who can reach the Mac, and the key decides who gets in. Tailscale Serve names the calling user only for personal devices, not for tagged devices, which servers often are, so Apple Bridge never relies on a Tailscale identity to accept an agent. An approved agent holds a key, and the gateway accepts the key, not the device.

**Over Tailscale.** `bridge tailscale` turns on the tailnet-only route and prints one sentence to give the agent: "Connect to Apple Bridge at https://<mac>.ts.net:8443/connect and use it from now on." The agent reads `/connect`, which lists the steps and ready commands for Claude Code, Codex and a plain MCP config. It posts to `/connect/request`, the owner clicks Allow once on the Mac, and `/connect/answer` returns the key together with the MCP address and the exact header to send. Nothing on the agent's side needs a browser. The panel names the agent and the device it asked from, which the gateway looks up from the source address with Tailscale's whois ("Claude Code on my-vps"); the requester cannot set the device name.

**Over plain SSH, or any other private network that carries SSH.** The agent's MCP command is `ssh -T <owner>@<mac> <bridge> mcp`. `bridge mcp` runs on the Mac and asks exactly as a local agent does. It names the agent by where the SSH connection came from ("Claude Code over SSH from my-vps") and keeps that key apart from local agents. SSH authenticates the server, and the Mac still asks the owner once.

**Allowed agents read and change.** An agent allowed through the panel gets read and write scopes for every agent source, which the panel already promised ("It can read and change everything"). Before, it got write scopes only and could not read.

**When the owner's Tailscale rules block a device.** A blocked TCP connection on a tailnet gets no answer; it just waits. `bridge tailscale` reads this Mac's packet filter (`debug-packet-filter-rules`) and peer list from the Tailscale app, names each device that cannot reach the route's port, and prints one `acls` rule with their addresses and this Mac's address and port, with a link to the policy file. When a blocked device can already reach the Mac over SSH, it prints the SSH MCP command instead. Apple Bridge never changes the owner's rules.

No public address and no relay. ADR 0032's list of who may create an agent request stays the connector socket and the Tailscale route.
