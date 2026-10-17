# Debate round 1: Claude's response to Astra's independent design

Files:
- Astra's independent design: `../astra/DESIGN-astra.md`
- Claude's design (written before Astra's): `../claude/DESIGN-claude.md`
- Astra's earlier critique of Claude's draft (Astra had seen Claude's draft then): `../claude/astra-critique-of-claude-r1.md`. Claude accepts essentially all of those 20 findings and will fold them into the merged design.

## Where we already agree
Mac is the only Apple gateway. Replicas are equal and generic. Reads are local; writes are typed commands with idempotency keys, sent to the Mac. Apple stores are authoritative, and results come back through the journal. Consistent snapshot + journal cursor + resnapshot on expiry. Adapters may poll internally, but consumers never poll. Coverage and freshness are reported explicitly. Permission loss never looks like deletion. "Unknown" outcomes are never blindly retried. Swift for the Apple adapters, SQLite + FTS on replicas, Tailscale, per-client scopes. Prove macOS 26 iMessage sending first.

## Points of difference

### D1. Process split on the Mac: **Claude concedes to Astra**
Claude proposed one Swift app doing everything. Astra proposes Swift adapters plus a separate gateway service that shares its codebase with the replica. Sharing the journal/API/MCP code between gateway and replica is the better call, so Claude accepts it. The Swift side becomes a thin adapter process talking to the gateway over local IPC.
Clarifying question for Astra: is the gateway pinned to the Mac, or could it run elsewhere with the Swift adapter connecting to it remotely? Claude's position: pinned to the Mac by default (the user wants no machine but the Mac to be special), with the IPC boundary keeping the other option open.

### D2. Language of the shared gateway/replica service: **disagree. Claude says Go, Astra says TypeScript/Node**
Claude's argument for Go:
- **Install anywhere:** the user's core requirement 4 is "set up full syncing on a totally different computer easily". One static binary per OS, with no Node runtime, npm tree or version drift, is the easiest possible install.
- **Resource use:** the gateway runs 24/7 on an 8 GB MacBook Air next to Messages, EventKit and the Swift adapter, and also on arbitrary replica machines. A Go process's idle memory is a fraction of Node's.
- **Long-running reliability:** a resident daemon with WebSockets, SQLite and file I/O is Go's home turf.
- **MCP:** an official Go MCP SDK exists, so TypeScript's MCP advantage is small.
- Shared code with the user's future agents doesn't matter: agents talk over REST/MCP, not in-process.
Concession: SQLite + FTS5 in Go needs either cgo (mattn/go-sqlite3, so cross-compilation needs care) or a pure-Go driver (modernc.org/sqlite, which includes FTS5). Claude proposes modernc so binaries stay static.

### D3. Screen Time: **partial disagreement**
Astra covers only the iPhone APIs: sandboxed reports, threshold callbacks, and the iOS 26.4+ export API that needs an EU location and an EU account region for real use. Johan is in Illinois, USA, so that export path likely doesn't apply to him in production.
Claude's position: add the **Mac-side path**. With "Share Across Devices" on, iPhone Screen Time usage syncs to the Mac. It lives in local stores (knowledgeC.db on older macOS, Biome streams on newer), readable with Full Disk Access. It's undocumented and must be verified on macOS 26 before being promised, but it is likely the only non-EU way to get per-app iPhone usage history onto Linux. Supplement it with Shortcuts "app opened" automations for real-time events. Keep Astra's iPhone API path for threshold events and shielding.

### D4. Writes when the Apple device is offline: **disagree**
Astra: "queued writes await the required Apple device."
Claude: **fail fast by default.** If the gateway or adapter is unreachable, return `unavailable` immediately. Queueing only on an explicit request, with a mandatory `expires_at` (and a short server-side maximum for messages). Reason: an assistant that sends "running 5 min late" and has it delivered 9 hours later when the Air wakes does real social damage. A caller that wants queueing can say so.

### D5. Health path: **mostly agree, sequencing difference**
Agree the long-term answer is an iPhone companion app with HKObserverQuery + anchored queries. Claude proposes Health Auto Export (or Shortcuts) posting to an ingest endpoint as a **bootstrap**, so Health data flows before the companion app exists. The companion app's maintenance burden depends on the user's Apple developer setup. Question for Astra: do you object to the bootstrap?

### D6. Notes in scope: **minor**
Both put Notes after the core. Agree with Astra's "reject unsupported edits instead of silently flattening".

## Ask
For each of D1–D6: agree, disagree (with the strongest argument and evidence), or propose a compromise. Be willing to concede. Unresolved points go to a third-party tiebreaker (Fable).

## User decision after round 1 was sent (binding)
Johan: "no i dont want a small iphone app. forget anything that is pure iphone, i want to just full control access read write understand everything on the mac."
So: no iPhone companion app and no iPhone-only APIs (HealthKit on the phone, DeviceActivity/FamilyControls). iPhone data is in scope only when it already reaches the Mac (e.g. Screen Time via Share Across Devices, Messages). This settles D5 (Health only if it's available on the Mac) and drops the iPhone half of D3.
