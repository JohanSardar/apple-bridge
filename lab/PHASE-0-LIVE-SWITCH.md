# Phase 0 live remote switch

impl=go. Executed 2026-11-02 on the test Mac using the existing live Go gateway, file secret store and unchanged signed Helper. No permission prompts or interactive login were needed. No personal hostnames, addresses, credentials or records are included here.

`tailscale up` exited 0. `bridge remote on --via tailscale-app` exited 0 after the built-in public HTTPS/OAuth/MCP and 60-second SSE qualification. Persisted backend is `system` and state is `on`. The existing relay enrollment file remains present. The active gateway supports one remote backend at a time (`internal/remote/backend.go`); selecting system does not also establish a relay tunnel. A probe of the retained relay hostname failed TLS with `UNEXPECTED_EOF_WHILE_READING`. The orchestrator confirmed this is the expected pre-switch condition: private mode did not serve the relay hostname either. This is not a regression. Rust ab-remote should support relay and the Tailscale app concurrently instead of carrying forward this exclusive selection. Do not route sensitive replicas over relay/1; R0 containment is separate work.

The system remote URL answered HTTP 401 from the test Mac, proving HTTPS reachability without sending credentials. A direct tailnet HTTPS probe from Ubuntu timed out (curl exit 28, status 000); remote tailnet reachability remains unverified and is left to the R0 routing worker as instructed. Source inspection used SQLite mode=ro: 18 sources ready (messages, contacts, calendar, reminders, notes, usage, pdf, mail, mac, focus, location, maps, speech, capture, shortcuts, vision, phone, files), five degraded (photos, voice_memos, safari, music, voicemail). The degraded sources reported existing capture failures/cloud availability; the orchestrator adopted this list as the baseline from this point forward. No source restart or permission grant was attempted.

Rollback on the test Mac:

```sh
APPLE_BRIDGE_SECRET_STORE=file ~/apple-bridge-private/main-10a397d6/bridge remote on --home "$HOME/Library/Application Support/AppleBridgeLive" --via private
```

To select the retained relay enrollment instead of system, use the same command with `--via relay`. This is an alternative backend, not simultaneous relay plus system service. The one-week access-path soak remains pending; ADR 0021 does not claim it has completed. Disk inspection found about 11 GiB free, below the 25 GB prerequisite. No pruning was authorized or performed in this task.
