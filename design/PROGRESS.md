# Progress

Resume point for any fresh session. Updated after every unit of work.

## Environment facts (verified 2026-10-17)
- Air: macOS 26.7.1 (25G241), SIP on, 8 GB, Swift 6.3.3 CLT, Go 1.27.2 (Homebrew), ~15 GB free disk (tight).
- SSH session on the Air has Full Disk Access: chat.db (WAL), NoteStore.sqlite, Biome restricted streams readable.
- chat.db: services iMessage, SMS, RCS present; schema includes date_edited, date_retracted, message_summary_info, associated_message_emoji.
- No code-signing identities on the Air (`security find-identity` = 0). notarytool/stapler exist in CLT. Developer ID needed for M5 notarization.
- Air → Ubuntu: `ssh ubuntu` works (key air_to_ubuntu). Ubuntu → Air: `ssh air`. Tailscale: Air 100.78.200.121, Ubuntu 100.80.96.36.
- Ubuntu: no Go (replica binaries are cross-compiled on the Air), codex-cli 0.163.0 present.
- Biome restricted streams on this build: App.InFocus, ScreenTime.AppUsage, UserFocus.*, Screen.Sharing; no Display/ScreenLock stream found by name (M4 must find clipping evidence elsewhere).

## Done
- Step 1: design/SPEC.md, GLOSSARY.md, PRINCIPLES.md, docs/adr/0001–0014, features.yaml, tools/featuregraph (checker + HTML), tools/lint (zero comments + import boundaries), Makefile.

## Verified
- `go run ./tools/lint` ok; `go run ./tools/featuregraph` ok (24 features, 0/177 scenario-levels passing).

## Next
- M0: harness skeleton + SimApplePeer + State/Replication/Operations/Trust/Frontdoor at sim level.

## Blockers
- None yet. Expected: TCC permission batch for the Helper (M1), Developer ID for notarization (M5).
