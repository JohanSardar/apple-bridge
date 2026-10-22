# Rules for every worker (binding)

You are a worker on Apple Bridge, delegated by the build leader. Read this file, then `design/SPEC.md` (binding), `GLOSSARY.md`, `PRINCIPLES.md`, `docs/adr/`, `features.yaml`, and `design/PROGRESS.md`.

## Where things run
- You run on the Ubuntu laptop. The repo lives on the MacBook Air at `~/apple-bridge` and is mounted at `/home/johan/air/apple-bridge` (sshfs). Edit files through the mount.
- If ssh to the Air hangs, the shared ControlMaster socket is broken: use `ssh -o ControlPath=none air` (always safe). The `air` alias tries the Air's direct LAN address first and falls back to the reverse tunnel (127.0.0.1:2222), whose launchd job on the Air can restart. The Tailscale IP may be blocked by an ACL; prefer the `air` alias.
- **Don't end your turn while your own work is pending.** If a detached job is queued or running, or a planned PR in your unit isn't merged, keep going: check logs every few minutes and write the next item's code while a step waits in the gate. Nothing resumes a worker whose turn has ended. End the turn only when you're blocked on something you can't do yourself, or when your unit is done.
- **Run long jobs detached on the Air** so a dropped ssh session can't kill them: `ssh air 'cd <worktree> && nohup packaging/dev/heavy.sh <cmd> > /tmp/<name>.log 2>&1 & echo $!'`, then poll the log or PID. Kill your own leftover processes and remove their `/tmp/ablab/<world>` when you abandon a run.
- Run **every** build, test, git and service command on the Air: `ssh air 'export PATH=/opt/homebrew/bin:$HOME/go/bin:$PATH; cd ~/apple-bridge && <command>'`. Never run git over the mount. Never build on Ubuntu (there is no Go there).
- If the mount drops: `sshfs -o reconnect,ServerAliveInterval=15 air:/Users/johansardar/apple-bridge /home/johan/air/apple-bridge`. If `air` doesn't resolve, use `100.78.200.121`.
- The Air has 8 GB RAM and ~15 GB free disk. Another worker may be building at the same time: never run more than one heavy build/test at once yourself, use `-p 2` for `go test`/`go build` when heavy, and clean temp dirs you create (`/tmp/ablab/...`).
- The Ubuntu laptop is the "other device" for replica tests: the Air can `ssh ubuntu`. Replica binaries for Ubuntu are cross-compiled on the Air (`GOOS=linux GOARCH=amd64`) and copied with `scp` from the Air to `ubuntu:/tmp/ablab/`.

## How to build
- E2E first: every feature starts as a failing scenario in `lab/scenarios/`, run by `bridge-lab`. Code-level tests only for decoders, time math and crash/race timing unreachable from outside. Do not write unit tests for behaviour testable end to end.
- **Zero comments** in Go and Swift (including tests). `make lint` must pass. Explain with names and types.
- Deep modules with small interfaces as in SPEC §5; respect import boundaries (lint enforces). Use the glossary's words in code.
- Keep dependencies minimal and permissively licensed (allowed: `modernc.org/sqlite`, `github.com/coder/websocket`, `github.com/modelcontextprotocol/go-sdk`, `gopkg.in/yaml.v3`; ask the leader in your report before adding others).
- `gofmt`, `go vet`, `make lint`, `make features` must pass before you report.

## Safety
- Never text anyone but Johan himself; group tests may include Ben (handle in `~/.config/apple-bridge/test-contacts.env` on the Air — read it at runtime, never print it, never commit it).
- Never modify or delete Johan's real data outside things tests created. Tag test data ("Bridge Test" calendar/list/folder/organization, `[ab-test <id>]` in message text) and clean it up.
- SIP stays on. No private-API injection. Don't change system security settings. Don't touch `~/toy`, Hermes or other projects on either machine.
- Test cleanup may delete a whole container (calendar, list, folder, group) only if this run recorded creating it AND an unbounded enumeration (no date window, completed items included) proves every item in it is tagged by this run. Otherwise delete individually tagged items only. Never delete or modify any container or item the tests didn't create.
- Never commit personal data (phone numbers, emails, message text, usage numbers, IPs, tokens). Evidence files are redacted.
- A macOS permission dialog, password or physical action needed from Johan is a blocker: stop that line of work, report exactly what is needed, continue with everything else.

## Git and GitHub (Johan's rules, binding)
- **Branch:** one branch per slice, named `feat/…`, `fix/…`, `perf/…`, `refactor/…`, `docs/…`, `test/…` or `chore/…`. Each lives in your own worktree on the Air.
- **Commits:**
  - **Normal, professional size (Johan's rule since 2026-10-21 evening):** one commit per complete logical change. The implementation goes together with its tests, the scenario and evidence it owns, and its doc or SPEC lines, typically 100–800 changed lines. A PR usually has 1–3 commits.
  - No separate commits for evidence-only updates, fixture tweaks, review fixes, formatting or "checkpoints". Fold them into the commit they belong to (`git commit --fixup` plus `git rebase -i --autosquash`, or amend) before pushing or merging. Squash local work-in-progress commits.
  - Each commit builds and passes the relevant tests.
  - Conventional Commit messages (`feat(scope): …`, `fix(scope): …`), ending with `Co-Authored-By: GPT-6.1-Sol <noreply@openai.com>`.
- **Identity:** the GitHub noreply address set in the repo config. Never change it.
- **Before every commit,** check `git diff --cached` for secrets and personal data: tokens, keys, Tailscale state, real phone numbers, emails other than @example.com, real message text. Unstage anything found and gitignore it.
- **Shipping a slice** (usually 1–3 commits) as soon as it works:
  1. `git push -u origin <branch>`
  2. `gh pr create --base main --title "<conventional title>" --body "<what changed; how tested: exact commands and results>"`
  3. Run the slice's tests and linters.
  4. `gh pr merge <n> --merge --delete-branch`
  5. Update the shared checkout: `ssh air 'cd ~/apple-bridge && git checkout main && git fetch origin main && git merge --ff-only origin/main'`, only when it's clean.
  6. `git fetch origin && git rebase origin/main` before the next slice.
- **Updating the shared checkout:** use `git fetch origin main && git merge --ff-only origin/main`, not `git pull`. All worktrees share one `.git/FETCH_HEAD`, so concurrent fetches by other workers can make `git pull` fail with "Cannot fast-forward to multiple branches".
- **Size:** one PR per feature or fix, a reviewable unit of roughly 200–1,500 lines. Put a fix and the test-harness change it needs in the same PR, not two. Never one giant PR that mixes unrelated features.
- **Evidence ownership:** a PR commits `lab/evidence/**` files only for the scenarios that PR adds or changes. Regression runs rewrite other scenarios' evidence; restore those before committing (`git checkout origin/main -- <paths>`) and quote the regression results in the PR body instead. When a rebase still conflicts on evidence JSON, keep `origin/main`'s version for scenarios you don't own.
- **Red main:** if main is red, fixing it comes first.
- **No padding:** no empty, whitespace-only or count-inflating commits.
- **Reporting:** put every PR URL in your progress messages and final report.
- **No force-pushing** to `main`.
- **Leader-owned files:** don't edit `design/SPEC.md`, `features.yaml` statuses or `design/PROGRESS.md` unless your brief says so; propose spec changes in your report. You may add new scenarios to `lab/scenarios/` and list them in your report when you find an untested behaviour.

## Report back (final message)
1. What you built (files/modules) and commits (SHAs).
2. Exact commands you ran to verify and their results (scenario ids, levels, pass/fail, latency p50/p95/max).
3. Deviations from SPEC and why; proposed spec changes.
4. New scenarios you added.
5. Open problems, blockers, and what Johan must do (if anything).

## Helper signing identity (do not break)
- The dev Helper `~/Applications/AppleBridgeDev/AppleBridgeHelper.app` (bundle id `dev.applebridge.helper`) is signed by the dedicated self-signed certificate in `~/Library/Keychains/apple-bridge-dev.keychain-db` via `packaging/dev/sign.sh`. Johan grants TCC permissions to it once. Never regenerate or replace that certificate, never change the bundle id or the designated requirement, and always sign through `packaging/dev/sign.sh`; otherwise every grant is lost.
- Check permissions without prompting: `open -W -a ~/Applications/AppleBridgeDev/AppleBridgeHelper.app --args status --report "$HOME/Library/Application Support/AppleBridgeDev/status.json"`. Never run `onboard` unless the leader says so.

## Parallel work (several workers run at once)
- Work in your own git worktree on the Air, never in the shared checkout: `ssh air 'cd ~/apple-bridge && git fetch origin && git worktree add .worktrees/<your-unit> -b <type>/<slice> origin/main'`. Edit through the mount at `/home/johan/air/apple-bridge/.worktrees/<your-unit>/`. Build and test inside that directory on the Air.
- Wrap every heavy command (go build/test of many packages, bridge-lab runs, swift builds, Playwright) in `packaging/dev/heavy.sh <command...>` (from the repo root of your worktree). It runs one heavy command at a time on hosts with less than 12 GiB of RAM, such as the 8 GB Air, because two concurrent heavy jobs thrash swap there (`APPLE_BRIDGE_HEAVY_SLOTS` overrides this). Larger hosts allow 2, falling back to 1 while actually thrashing: more than 256 pages swapped out in a 2 s sample, a 1-minute load above twice the CPU count, or free memory below 20% (`kern.memorystatus_level`), and nested calls run inside the caller's slot. `make web` and `make build` already gate internally. Swift builds (`build-native.sh`, used by `build-helper.sh` and `build-truth.sh`) also take a Mac-wide lock, so only one Swift compile runs at a time. Run Playwright with `--workers=1` and focused subsets, and run the full 12-project matrix only right before merging.
- **Light lane.** Short, low-memory checks run in `packaging/dev/heavy.sh --light <cmd>`, a separate single slot that never waits behind long heavy jobs: `make features`, `gofmt -l`, `go run ./tools/lint`, `pnpm exec tsc --noEmit`, git/gh operations, and small Python/JSON checks. Anything that compiles Go packages (`go build`, `go vet`, `go test`, `make lint`), builds Swift, runs Playwright or runs `bridge-lab` stays in the normal heavy lane.
- **Sim lane.** A plain functional `bridge-lab run` at sim level (`--level sim` or no `--level`) automatically also queues for a second, single sim slot and runs in whichever lane frees first. The sim slot admits a job only while the Air has headroom: at least 45% free memory, a 1-minute load below the CPU count and almost no swapping. It never overlaps sensitive work: real or agent runs, `--require-quiet`, Swift builds, exclusive gates, or anything whose command names latency or perf or whose scenario YAML (passed as a file argument) mentions `latency` or `p95`. Sensitive normal-lane jobs wait for a running sim job to finish before they start. Commands that mention real, agent, quiet, latency or perf stay in the normal lane only. `--sim <cmd>` opts other light-to-medium Go work into the same lane. `APPLE_BRIDGE_HEAVY_NO_SIM_LANE=1` keeps a run in the normal lane and also isolates it from sim jobs, whether it is exported before `heavy.sh` or passed inside the command (`heavy.sh env APPLE_BRIDGE_HEAVY_NO_SIM_LANE=1 …`). Use it for timing gates whose command doesn't name latency or perf, and for sim runs that compile Swift.
- **Critical-path priority.** Only when the leader names your work the critical path, set `APPLE_BRIDGE_HEAVY_PRIORITY=1` on its heavy steps. They then go ahead of normal tickets in the FIFO, but still wait for the job that is running.
- **Dispatch-only rebases keep earlier greens.** Sometimes a rebase's only conflicts or changes are other PRs' lab dispatch or registration entries in `lab/scenarios/runner.go` or `validation.go`, and your product source is byte-identical. Show that with `git range-diff` and a product-source digest in the PR body. In that case your earlier green gates on that product source stay valid. Rerun only lint/vet, the Go build, features and your own scenarios whose dispatch lines the resolution touched. Any product-source change, or a conflict anywhere else, still requires the full final-tree batch.
- **Gate each heavy step on its own.** One `heavy.sh` invocation is one build, one lint/features/vet pass, or one scenario run (about 10 minutes or less). Batch scripts call `packaging/dev/heavy.sh <step>` for each step instead of wrapping the whole script, so the FIFO interleaves and short focused checks don't wait behind long batches. The only whole-batch exception is an exclusive performance invocation.
- **Performance gates run exclusively.** The browser performance gates `WEB-PERF-*`, `WEB-LIVE-DELTA`, `WEB-MSG-PERF` and `WEB-MSG-LIVE`, and only these, run in their own invocation under `packaging/dev/heavy.sh --exclusive …`, which waits in queue order and then holds every slot, waits for any running sim job, and also takes the light lane. Running light checks finish first and new ones wait until the gate ends, so short `tsc`/lint/features runs cannot overlap a measurement. When a budget fails inside a shared run, rerun that scenario once under `--exclusive`. The exclusive result is the authoritative measurement; record both, along with host load and swap. Never loosen a threshold. Everything else, including diagnostics and sim scenarios that report latency, uses the normal lane, because each exclusive job blocks the whole queue.
- **Size the pre-merge run to the change.** The full 12-project web matrix runs before merging PRs that change `web/`, `internal/web`, or the web runner. Other PRs run the scenarios for the code they touch, plus the core and parity suites they could affect. Everything runs at `--workers=1`.
- Integrate often: `git fetch origin && git rebase origin/main` in your worktree, rerun your scenarios, then ship through a PR as in the Git section. If the shared checkout has local changes, don't touch them; tell the leader. Remove your worktree when done (`git worktree remove`).
- Use unique world names and ports in lab runs (the World must allocate free ports; never hard-code ports that another worker's run could also use).

## GUI control of the Air
- `~/tools/vncenv/bin/python ~/tools/airvnc.py capture /tmp/x.png | click X Y | type "text" | key enter | pause 1.5` (chainable) drives the Air's real screen (2940x1912 framebuffer; downscale screenshots with Pillow from `~/tools/vncenv/bin/python` before viewing; scale clicks back). VNC input is real input: it can click TCC Allow buttons and System Settings toggles.
- Only use it when your brief assigns GUI work (the screen is shared; one GUI actor at a time). Always capture before acting and after to verify. Never type passwords. If the lock screen shows, wait and recheck; report if it persists.
- UI-automation scenarios (Accessibility actions in Messages) may use captures as Truth evidence for visible effects; keep screenshots in private, gitignored paths (they contain personal data).

- Real Worlds stage immutable, signed Helper and AppleTruth bundles under their own `/tmp/ablab/<world>/apps/` from the current worktree using `packaging/dev/build-helper.sh <destination.app>` and `packaging/dev/build-truth.sh <destination.app>`. Always pass a new destination; never rebuild a running World’s bundle or install into the shared dev apps during worker verification. Signing still uses `packaging/dev/sign.sh` and the existing certificate, bundle IDs and designated requirements. Launch with `open -n -W -a`.

## Speed rules (2026-10-17, binding)
- Reviews: only new units get one review, by one reviewer doing both axes (Standards + Spec) in a single pass. Never spawn review sub-agents. Fix rounds are not re-reviewed: the fixer proves each fix with its failing-then-passing scenario (or by reverting it locally to show the check fails).
- Merge first: merge to main as soon as the unit's scenarios pass; don't sit on branches. List anything unproven precisely instead of polishing.
- No gold-plating beyond DECISIONS.md and the 20 findings: no quiet-machine waits, no benchmarks beyond the <1 s latency checks, no doctor extras. Host telemetry stays one line in evidence, never a gate. The one exception is the `heavy.sh` thrash guard: sensitive jobs (real, latency, perf, exclusive, Swift) hold for up to 20 minutes while the Air is actually thrashing. That means a 1-minute load above 2x the CPUs (16 on the Air), free memory below 25%, or more than 2,048 swapped pages in 2 s, as caused by Johan's own 30-minute `com.excerlock.transcript-mirror` launchd job, which agents never touch. Other heavy jobs hold only through extreme thrash (load above 4x the CPUs, free memory below 15% or more than 8,192 swapped pages in 2 s), because they would mostly time out there anyway. The light lane never holds. Timing results that still overlap such a window are host-invalid and are rerun. Classify by who causes the load, using a top-process sample. Load from **non-product** processes (other workers' jobs, Johan's apps or launchd jobs, macOS updates) makes timing host-invalid. Load from **the product under test** (AppleBridgeHelper, the gateway/`bridge`, our Truth) is part of the system being measured: such a run is a valid result, and a miss is a real product RED.
- Commit at least every 2 hours of work; a worker with no commit for 3 hours is treated as stalled and gets re-scoped.
