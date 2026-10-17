# Poteto principles for Apple Bridge

## Sources and scope

Read **2026-10-16, America/New_York**. The [X post][post], dated September 21, opens: “here's how i shipped 2,500 PRs last month to production”. Remainder paraphrased: the recording was intended for Cursor Compile London; she missed it while livestreaming Grok Bot Galaxy, released it freely on X, and recommends double-speed playback.

Access attempts: X web fetch returned **403**; xcancel and FxTwitter were inaccessible through web fetch. Direct HTTP to [FxTwitter's API][fx] succeeded, returning post text, timestamp, and a 2,281.984-second video. X's syndication endpoint returned `{}`. GitHub HTML subtree access failed; GitHub API and raw files succeeded.

**The post contains a video, not a repository URL.** The strongest identified match is [cursor/plugins/pstack][repo]: its README identifies poteto as author, and her mirrored profile promotes pstack. This establishes association, not a direct repository link in the post. Her [repository listing][repos] also surfaced `noodle`, `how`, `brainmaxxing`, and the verification example; no evidence makes Noodle this post's target.

Repository snapshot: [`09d06eb4b97a1020ab31e84030e38baf1099c7dc`][commit]. Read pstack's README, all 24 principle files, guides 01–10, mode, comment policy, TDD, and related skills. The [tree][tree] contains no root/pstack `AGENTS.md` or `CLAUDE.md`.

Related material: Cursor's [official programme][compile] names the talk *I Shipped 2,000 PRs Last Month*. The post says 2,500; neither is independently verified here. A [transcript mirror][transcript] discusses verification, architecture, gardening, and comments. Poteto's [verification example][example] is explicitly fictional and omits driver implementations.

## Principles and concrete application

Each linked source identifies its file. Items 1–24 reproduce the complete formal inventory in paraphrase. Text after **Bridge** is my interpretation for the Swift helper, Go replica, REST/MCP, and AI workflow.

1. **Laziness protocol.** Centralize choices; minimize diffs. **Bridge:** one owner for sync policy, no duplicated adapter/service decisions. [Source][p1].
2. **Foundational thinking.** Data structures and enabling infrastructure first. **Bridge:** define identities, cursors, and transactions; scaffold runnable E2E verification before expanding adapters. [Source][p2].
3. **Redesign from first principles.** Integrate requirements into the underlying design. **Bridge:** represent deletion and resumption in the sync model, avoiding adapter-specific patches. [Source][p3].
4. **Attack the premise.** After repeated failed fixes, test their shared assumption. **Bridge:** investigate cursor semantics before adding another retry. [Source][p4].
5. **Subtract before adding.** Remove dead complexity and speculative defenses. **Bridge:** delete obsolete adapters and duplicate validators before extending sync. [Source][p5].
6. **Minimize reader load.** Reduce indirection and hidden mutable state. **Bridge:** keep adapter-to-replica paths traceable; avoid pass-through interfaces. [Source][p6].
7. **Outcome-oriented execution.** Converge on a verified target; permit scoped, reversible transitional breakage. **Bridge:** declare migration checkpoints instead of accumulating compatibility scaffolding. [Source][p7].
8. **Experience first.** Favor consumer outcomes and polished essentials. **Bridge:** prioritize reliable data access, permission feedback, and actionable REST/MCP errors. [Source][p8].
9. **Exhaust the design space.** Compare competing runnable prototypes for novel decisions. **Bridge:** evaluate transport/storage alternatives on a real vertical slice. [Source][p9].
10. **Build the lever.** Produce rerunnable tools instead of manual ceremony. **Bridge:** create a small verification CLI for seeding, syncing, querying, restarting, and cleanup. [Source][p10].
11. **Model the domain.** Encode rules in structures. **Bridge:** explicit adapter lifecycle and sync state machines replace interacting booleans. [Source][p11].
12. **Boundary discipline.** Validate external inputs once; keep domain logic pure. **Bridge:** parse Apple/IPC/HTTP/MCP inputs at their boundaries; propagate typed errors internally. [Source][p12].
13. **Type-system discipline.** Distinguish semantic primitives and derive authoritative shapes. **Bridge:** separate source IDs, replica IDs, and cursors; generate shared wire contracts. [Source][p13].
14. **Make operations idempotent.** Repetition and interrupted runs converge. **Bridge:** replay batches without duplicates; test crashes around record/checkpoint commits. [Source][p14].
15. **Migrate callers, delete legacy APIs.** Coordinate internal changes without permanent shims. **Bridge:** migrate owned callers and remove old helper/service interfaces together where deployment permits. [Source][p15].
16. **Separate before serializing shared state.** Eliminate sharing before adding locks. **Bridge:** isolated agent worktrees/databases; adapter-owned cursors; structural serialization for genuinely shared replica writes. [Source][p16].
17. **Prove it works.** Inspect the actual artifact. **Bridge:** run Swift extraction through Go ingestion, then read persisted results through REST and MCP after restart. [Source][p17].
18. **Fix root causes.** Reproduce and trace the earliest fault. **Bridge:** correct checkpoint advancement rather than hiding missing records with retries. [Source][p18].
19. **Sequence verifiable units.** Check small increments before continuing. **Bridge:** land one complete adapter journey at a time with reproducible evidence. [Source][p19].
20. **Test behavior, not implementation.** Assertions survive internal refactoring. **Bridge:** assert observable records, errors, persistence, and duplicate prevention; avoid private-helper expectations. [Source][p20].
21. **Explain the number.** Establish bottlenecks, repeated measurements, and validity. **Bridge:** benchmark complete extraction-to-query latency, checking missing work, caching, errors, and run variance. [Source][p21].
22. **Guard the context window.** Bound scope; delegate bulk investigation and summarize. **Bridge:** supply agents focused adapter/contracts and evidence pointers. [Source][p22].
23. **Never block on the human.** Proceed with reversible work; preserve irreversible-action gates. **Bridge:** agents implement and verify autonomously within explicit permissions. [Source][p23].
24. **Encode lessons in structure.** Prefer enforcement over repeated instructions. **Bridge:** turn recurring agent mistakes into types, lint, checks, or executable regressions. [Source][p24].

Additional philosophies in the supporting documentation:

25. **Quality precedes throughput.** Less, better code enables trusted parallelism. **Bridge:** optimize reliable outcomes rather than PR or line counts. [README.md][readme].
26. **Remove comments that excuse complexity.** Reshape code instead. **Bridge:** self-explanatory names/types; retain justified public contracts, licenses, and unavoidable Apple-platform constraints. [comment-sicko.md][comments], [no-comments/SKILL.md][no-comments].
27. **Understand before editing.** Separate mechanics, history, and inference. **Bridge:** trace Apple permissions and sync ownership before changing behavior. [03-understand.md][understand].
28. **Plan through code and caller usage.** Prototype uncertainties; write API tutorials first; delete completed plans. **Bridge:** demonstrate one helper-to-query journey before broad architecture commitments. [04-design.md][design].
29. **Verification is maintained infrastructure.** Repeatable environments, feature maps, health checks, evidence, cleanup, and composable JSON-capable control tools. **Bridge:** build and maintain `verify-apple-bridge`; include destructive-command dry runs. [06-verify-and-ship.md][verify].
30. **Independent, skeptical review.** Different agents judge changes; reject disproven findings; verify stacks bottom-up. **Bridge:** fresh reviewers run real macOS and REST/MCP paths before landing. [04-design.md][design], [06-verify-and-ship.md][verify].
31. **Earn autonomy.** Define finish conditions, permissions, escape hatches, and decision trails. **Bridge:** unattended runs stop on genuine blockers and preserve evidence without relaxing acceptance. [07-overnight.md][overnight].
32. **Small, personalized, evaluated skills.** Add workflows from observed failures; validate authoring and test changes blind. **Bridge:** evaluate agent instructions separately from product changes. [09-make-it-yours.md][personal].
33. **Clear, grounded communication.** State consumer/maintainer impact; distinguish evidence and inference; avoid padded prose. **Bridge:** report exact checks, results, and limitations with every delivery. [poteto-mode/SKILL.md][mode].
34. **Explicit ownership and isolation.** Coordinators direct scoped workers; machines/worktrees isolate concurrent runs; models fit roles. **Bridge:** isolate Swift and Go work, with macOS-capable workers for Apple verification. [02-poteto-mode.md][routing], [07-overnight.md][overnight], [README.md][readme].
35. **Conditional TDD.** Capture failing-before/passing-after behavior when cheap; reject brittle or disproportionate tests. **Bridge:** use focused regression checks when they add signal. [tdd/SKILL.md][tdd].
36. **Garden the codebase as agent memory.** The talk describes agents repeating visible patterns. **Bridge:** remove bad examples and enforce replacements through architecture and CI. [Transcript mirror][transcript], [09-make-it-yours.md][personal].

## Tensions and recommended adaptations

- **E2E-first fits; an absolute unit-test ban does not.** TDD explicitly favors the narrowest practical existing unit/component/integration check. Keep E2E acceptance central; allow cheap pure-logic regressions. Skip mock-heavy tests and expensive harness construction for small fixes. [TDD][tdd].
- **Small diffs versus redesign/prototypes/tool building:** scale investment to observed complexity, preserving the smallest coherent solution. [Laziness][p1], [Redesign][p3], [Lever][p10].
- **Green increments versus transitional breakage:** migration phases may break internally; declared delivery checkpoints require full verification. [Sequencing][p19], [Outcomes][p7].
- **Immediate legacy deletion versus independent deployment:** the rule assumes no external compatibility obligation. Version Swift/Go protocols and REST/MCP contracts when consumers cannot migrate together. [Migration][p15].
- **No comments versus expressible constraints:** the repository explicitly keeps public contracts and proven external gotchas. A literal zero-comment policy is stricter. [Reviewer][comments].
- **Autonomy rules disagree:** the principle requires confirmation for external messages; mode permits team chat/ticket actions while pausing customer messages. Adopt explicit project permissions. [Principle][p23], [Mode][mode].

[post]: https://x.com/poteto/status/2102050467505430555?s=20
[fx]: https://api.fxtwitter.com/status/2102050467505430555
[repo]: https://github.com/cursor/plugins/tree/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack
[repos]: https://api.github.com/users/poteto/repos?sort=updated&per_page=15
[commit]: https://github.com/cursor/plugins/commit/09d06eb4b97a1020ab31e84030e38baf1099c7dc
[tree]: https://api.github.com/repos/cursor/plugins/git/trees/09d06eb4b97a1020ab31e84030e38baf1099c7dc?recursive=1
[compile]: https://cursor.com/compile/london
[transcript]: https://youtube-distilled.com/watch/NjoZoUm85x0
[example]: https://github.com/poteto/verification-skill-example/blob/d5abe70d0d8c671672b6cef4069363f26c488feb/README.md
[p1]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-laziness-protocol/SKILL.md
[p2]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-foundational-thinking/SKILL.md
[p3]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-redesign-from-first-principles/SKILL.md
[p4]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-attack-the-premise/SKILL.md
[p5]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-subtract-before-you-add/SKILL.md
[p6]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-minimize-reader-load/SKILL.md
[p7]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-outcome-oriented-execution/SKILL.md
[p8]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-experience-first/SKILL.md
[p9]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-exhaust-the-design-space/SKILL.md
[p10]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-build-the-lever/SKILL.md
[p11]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-model-the-domain/SKILL.md
[p12]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-boundary-discipline/SKILL.md
[p13]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-type-system-discipline/SKILL.md
[p14]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-make-operations-idempotent/SKILL.md
[p15]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-migrate-callers-then-delete-legacy-apis/SKILL.md
[p16]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-separate-before-serializing-shared-state/SKILL.md
[p17]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-prove-it-works/SKILL.md
[p18]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-fix-root-causes/SKILL.md
[p19]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-sequence-verifiable-units/SKILL.md
[p20]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-test-behavior-not-implementation/SKILL.md
[p21]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-explain-the-number/SKILL.md
[p22]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-guard-the-context-window/SKILL.md
[p23]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-never-block-on-the-human/SKILL.md
[p24]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/principle-encode-lessons-in-structure/SKILL.md
[readme]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/README.md
[comments]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/agents/comment-sicko.md
[no-comments]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/no-comments/SKILL.md
[understand]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/03-understand.md
[design]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/04-design.md
[verify]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/06-verify-and-ship.md
[overnight]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/07-overnight.md
[personal]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/09-make-it-yours.md
[mode]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/poteto-mode/SKILL.md
[routing]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/docs/guide/02-poteto-mode.md
[tdd]: https://github.com/cursor/plugins/blob/09d06eb4b97a1020ab31e84030e38baf1099c7dc/pstack/skills/tdd/SKILL.md
