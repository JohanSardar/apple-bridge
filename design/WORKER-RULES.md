# Rules for every worker (binding)

You are a worker on Apple Bridge, delegated by the build leader. Read this file, then `design/SPEC.md` (binding), `GLOSSARY.md`, `PRINCIPLES.md`, `docs/adr/`, `features.yaml`, and `design/PROGRESS.md`.

## Where things run
- You run on the Ubuntu laptop. The repo lives on the MacBook Air at `~/apple-bridge` and is mounted at `/home/johan/air/apple-bridge` (sshfs). Edit files through the mount.
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
- Never commit personal data (phone numbers, emails, message text, usage numbers, IPs, tokens). Evidence files are redacted.
- A macOS permission dialog, password or physical action needed from Johan is a blocker: stop that line of work, report exactly what is needed, continue with everything else.

## Git
- Commit small, verifiable steps on `main` from the Air (`git -C ~/apple-bridge ...`). Pull/rebase before pushing if needed; never force-push. Commit messages end with `Co-Authored-By: GPT-6.1-Sol <noreply@openai.com>`.
- Do not edit `design/SPEC.md`, `features.yaml` statuses, or `design/PROGRESS.md` unless your brief says so; propose spec changes in your report. You may add new scenarios to `lab/scenarios/` and list them in your report when you find an untested behaviour.

## Report back (final message)
1. What you built (files/modules) and commits (SHAs).
2. Exact commands you ran to verify and their results (scenario ids, levels, pass/fail, latency p50/p95/max).
3. Deviations from SPEC and why; proposed spec changes.
4. New scenarios you added.
5. Open problems, blockers, and what Johan must do (if anything).

## Helper signing identity (do not break)
- The dev Helper `~/Applications/AppleBridgeDev/AppleBridgeHelper.app` (bundle id `dev.applebridge.helper`) is signed by the dedicated self-signed certificate in `~/Library/Keychains/apple-bridge-dev.keychain-db` via `packaging/dev/sign.sh`. Johan grants TCC permissions to it once. Never regenerate or replace that certificate, never change the bundle id or the designated requirement, and always sign through `packaging/dev/sign.sh`; otherwise every grant is lost.
- Check permissions without prompting: `open -W -a ~/Applications/AppleBridgeDev/AppleBridgeHelper.app --args status --report "$HOME/Library/Application Support/AppleBridgeDev/status.json"`. Never run `onboard` unless the leader says so.
