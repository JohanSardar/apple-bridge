# Module architecture resolution (2026-10-16)

Rounds: independent designs (Astra, Claude) → Claude r1 → Astra r1 → Claude r2 (this file). Nothing unresolved, so no Fable tiebreak was needed.

Base: Astra's `../astra/MODULES-astra.md`, with these outcomes:

| # | Outcome |
|---|---|
| C1 | Prepare → Dispatch with single-use fenced tickets. A Swift dispatch-start guard survives helper restarts. A gateway restore rotates the command-admission era (old IDs are lookup-only). The full ledger is retained independently of mirror rebuilds. |
| C2 | One deep `State` module (records, aliases, journal, checkpoints, command transitions). |
| C3 | `Operations` owns ingestion and command orchestration. Native validation stays in Sources. `Runtime` owns process supervision. |
| C4/C9 | `capture(resume)`. `Ready` carries initial coverage; later permission/capability changes flow into State's journal. A failed capture never implies an empty source. |
| C5–C7 | UI actor for Accessibility. Feed with subscriptionRevision, `(after, through]`, credits (provisional budgets). `Trust` with Keychain / Linux secret-store adapters and delegated identity. |
| C8 | `Runtime` verbs are lifecycle behaviour, not a product CLI. Pairing may use onboarding UI. Only `bridge-lab` is an explicit control-tool interface. |
| C10 | Astra's repo layout. |
| D1 | **Separate `Calendar` and `Reminders` Sources** (Astra conceded), sharing an internal EventKit gate, each with its own checkpoints and scan-completion evidence. |
| D2 | **Handwritten operation descriptors** (name, shapes, versions, scope/capability, description) generate REST, MCP and OpenAPI registration. Frontdoor binds IDs to Operations; Contracts never imports Operations; Trust enforces at dispatch. Sequencing: types/codecs/validation and a small operation table first; broaden generation only after one complete path passes failure tests (findings 15/20). The same behaviour scenarios run through both REST and MCP. |
| D3 | **`lab/truth`** behind `Control.assert`, adapters SimTruth / AppleTruth. AppleTruth is a separate Swift target (`lab/truth/native`) sharing no helper code; UI screenshots where disk state can't prove a visible effect. An unavailable check reports `unverified`. |
| D4 | Harness internals: World, SimApplePeer, Truth, Scenarios (YAML describes existing harness actions, not a workflow language), AgentRunner, FeatureGraph. Assertion corrections: exactly-once applies to **replica application**, not Apple effects (command scenarios assert no automatic redispatch and truthful indeterminate status). Convergence = equality within declared coverage and authorization. Latency uses named endpoints on one monotonic observer clock, counts timeouts, and separates source-arrival delay, bridge propagation and blob completion. |
| D5 | **Claude concedes to Astra:** Blobs interface is `Stage`, `OpenAuthorized(recordGrant, digest)`, `Transfer`, `Protect(referenceSet)`, `Progress`. Digest knowledge never authorizes access; State supplies record/snapshot/command reference sets; pins bridge staging to durable protection. |
