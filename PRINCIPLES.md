# Principles

Johan's rules win where they conflict with anything below. Sources: `design/DECISIONS.md`, `design/BUILD-GOAL.md`, `research/poteto-principles.md`, Matt Pocock's engineering skills.

1. **E2E first.** Every feature starts as a failing end-to-end scenario at the sim level, then passes at the real level. Code-level tests only for decoders, time math and unreachable crash/race timing.
2. **Evidence over claims.** Truth decides; an unverifiable check is `unverified`, never a pass. Report exact checks and numbers.
3. **Deep modules, small interfaces.** Behaviour lives behind few entry points. One adapter is a hypothetical seam; two is a real one. Delete pass-throughs.
4. **Zero comments.** Names, types, the glossary and ADRs explain the code. Enforced by `make lint`.
5. **Model the domain.** State machines and typed IDs instead of interacting booleans and raw strings. Use the glossary's words in code.
6. **Validate at the boundary once.** Apple, IPC, HTTP, MCP and feed inputs are parsed at their edge; inside, typed values and typed errors.
7. **Idempotent and resumable.** Replays converge; interrupted work resumes or is discarded, never half-applied.
8. **Never guess about effects.** Ambiguous Apple effects stay indeterminate; nothing redispatches them.
9. **No silent loss.** A failure never looks like an empty source; absence needs a complete scan.
10. **One owner per decision.** Operations owns meaning, Trust owns visibility, Swift owns native behaviour, Frontdoor only translates.
11. **Subtract before adding.** Remove speculative code and duplicate validators before extending.
12. **Small verifiable steps.** Land one complete journey at a time with reproducible evidence; review every change with a different agent who reruns the scenarios.
13. **Fix root causes.** Reproduce, trace to the earliest fault, add the scenario that would have caught it.
14. **Explain the number.** Latency and resource numbers come with endpoints, sample counts, p50/p95/max and conditions.
15. **Encode lessons in structure.** Recurring mistakes become types, lint rules or scenarios.
16. **Safety with real data.** Text only Johan; tag and clean up test data; never touch data the tests didn't create; no personal data in git.
