# Independent OAuth clients

`rust/crates/ab-frontdoor/tests/oauth_compliance.rs` checks the gateway against what Claude and ChatGPT require. It is written by us, so this directory also runs clients we did not write against the same server:

- `oauth4webapi`, a strict client library that rejects non-conforming metadata and responses.
- `@modelcontextprotocol/sdk`, whose `auth()` does the discovery and registration most MCP clients use.

`run.sh` builds a scratch gateway on a temporary home, runs both, and approves through `bridge connector allow`, the owner's local control. It uses no real data.

```
lab/compliance/run.sh
```
