# Module debate round 1: Astra's response

Response to [Claude's round 1](modules-claude-r1.md), comparing [Astra](../astra/MODULES-astra.md) and [Claude](../claude/MODULES-claude.md). [Decisions](../DECISIONS.md), [resolution](RESOLUTION.md), and the [20 findings](../claude/astra-critique-of-claude-r1.md) remain binding.

## Clarifications to C1–C10

- **C1:** Correct, provided the dispatch-start guard survives helper restarts. Fenced tickets alone do not prevent replay after restoring an older gateway backup: rotate the command-admission era, allowing earlier IDs only for lookup. The gateway retains the complete ledger independently of mirror rebuilds.
- **C3:** Operations owns ingestion and command orchestration; native validation remains in Sources. Runtime owns process supervision. Folding HelperLink must preserve those responsibilities.
- **C4/C9:** `Ready` carries initial coverage, not immutable coverage. Permission and capability changes must subsequently reach State's journal; a failed capture cannot imply an empty source.
- **C8:** Agreed on avoiding a product CLI. Runtime's verbs specify callable lifecycle behavior; pairing may use onboarding UI. Only `bridge-lab` needs the explicit control-tool interface.
- **C2, C5–C7, C10:** Accurately understood. The frame/credit budgets remain provisional. D1 below changes the accepted layout from `Agenda` to `Calendar` and `Reminders`.

## D1 — Separate Calendar and Reminders: agree

I concede. Independent authorization, coverage, and failure lifecycles outweigh sharing EventKit. Keep two Source adapters, with independent checkpoints and scan-completion evidence, sharing an internal EventKit gate for store access and invalidation/refetch. Calendar occurrence windows and Reminders completion/recurrence remain in their respective implementations.

This improves locality without inventing an EventKit substitution seam: the existing Source seam already has multiple adapters. My Agenda grouping could be made correct, but its interface makes callers learn two independent lifecycles behind one name. That reduces depth.

## D2 — Generated outer surface: accept the descriptor compromise

Claude is right that reviewed operation descriptors can eliminate duplicated registration lists. Accept handwritten descriptors specifying names, shapes, versions, scope/capability requirements, and descriptions; derive REST registration, MCP registration, and OpenAPI from them. Frontdoor binds operation IDs to Operations; Contracts never imports Operations. Authorization remains enforced by Trust, including dispatch-time checks.

My objection was to deriving useful operations from collection schemas. Descriptors do not invent search semantics, conflict rules, or good tool descriptions. Nor does generation make all drift impossible: transport conversion and error mapping can still diverge. Exercise the same behavioral scenarios through both interfaces.

Keep the existing sequencing constraint: findings **15/20** require one complete path passing failure tests before broad codegen. Start with types/codecs/validation and a small operation table; expand generation after that evidence. This accepts Claude's compromise without reversing the binding gate.

## D3 — Independent truth oracle: agree

Claude identified a real omission. Add `lab/truth` behind `Control.assert`, with **SimTruth / AppleTruth** adapters and `Observe(query) → Fact`. AppleTruth uses a separate Swift target under `lab/truth/native`, sharing no helper implementation, decoding, or normalization code. It reads native state directly and captures Messages UI evidence where disk state cannot establish the visible effect.

Keep assertions focused on independently observable facts rather than duplicating the whole helper. Sharing Apple's undocumented schema assumptions can still produce correlated mistakes; independent code is necessary evidence, not infallibility. An unavailable oracle check reports unverified. SimTruth checks the controlled fake world; it cannot certify Apple behavior. This adds a real seam while preserving the product interface as the test surface.

## D4 — Harness internals: agree, with assertion corrections

Accept World, SimApplePeer, Truth, Scenarios, AgentRunner, and FeatureGraph inside `Control`. Deep modules may contain internal modules; those names do not require six substitution interfaces. Apply the deletion test: retain a module when removing it redistributes process, oracle, or evidence logic across scenarios. YAML should describe existing harness actions, avoiding a general workflow language.

Correct the proposed automatic assertions: **exactly-once means transactional replica application, never Apple effects** (finding **3**). Command scenarios instead assert no automatic redispatch of ambiguous effects and truthful indeterminate status. Convergence means equality within declared coverage and authorization.

Latency evidence needs named endpoints on one monotonic observer clock, with timeouts/missing results counted. Separate source-arrival delay, bridge propagation, and blob completion. The [Air probe](../../research/screentime-probe-results.md) found approximately twenty-hour iPhone lag; fast replica delivery cannot establish subsecond phone capture. Grade both scripted and agent journeys on oracle facts and public reads.

## D5 — Blob authorization and lifetime: unresolved interface difference

Claude's `Put/Open/Has(hash)` plus `Want(hash)` omits caller obligations that my Blobs interface makes explicit. We already agree on scope enforcement; the disagreement is how the interface carries authorization and lifetime guarantees.

Digest knowledge must not authorize access. A snapshot may retain bytes after the live record disappears; a staged command may need them before any record references them. Internal reference counting alone does not specify protection across those transactions.

I propose `OpenAuthorized(recordGrant,digest)`, secure `Stage`, transfer/progress operations, and `Protect(referenceSet)`: State supplies complete record/snapshot/command references, with pins bridging staging and durable protection. Equivalent interfaces are welcome if their contracts expose these guarantees. Findings **14/16** require them. Keep one concrete storage implementation; this adds no hypothetical storage adapter.

## Unresolved

- **D5:** Astra requires record-authorized blob opens and explicit reference/pin lifetimes; Claude's documented interface remains hash-oriented, with scope policy stated separately and reference-counted GC hidden internally.
