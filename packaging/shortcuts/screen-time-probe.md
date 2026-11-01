# Screen Time probe

`USE-ACTION-PROBE` uses the read-only usage data the macOS bridge already captures. A missing “Get App & Website Data” Shortcuts action does not prevent the probe from returning real output.

The macOS backend is `UsageSource`: it reads the local and synced `App.InFocus` Biome streams, identifies devices from the sync database, and returns `usage.device` and `usage.app_day` records. These are estimated focus durations with provenance, decoder version, device freshness and uncertainty. They are not a claim of parity with the Screen Time settings report.

The real probe stages an independent read-only copy of the source streams, compares its day totals with the gateway, checks provenance and freshness, and verifies replication and MCP output. Evidence contains counts and pass/fail only. It does not import a shortcut, request authorization, or change usage data. Full Disk Access must already allow the signed Helper to read the streams.

The iPhone DeviceActivity report remains available inside the authorized iOS report extension. This macOS probe does not claim to export data from that privacy boundary or require an iPhone entitlement.

Run `go run ./cmd/bridge-lab run lab/scenarios/usage/USE-ACTION-PROBE.yaml --level sim` locally. Run the real scenario on the Air through `~/bin/ab-real F9 ship/f9c-shortcuts shortcuts USE-ACTION-PROBE`.
