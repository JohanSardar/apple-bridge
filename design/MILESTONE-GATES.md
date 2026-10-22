# App milestone gate inventory

Inventory at origin/main 2303b82a, 2026-10-22. This is a run plan, not passing evidence. Private reports and screenshot sets stay in `lab/evidence/private/`.

Every functional web scenario runs WebKit, Chromium and Firefox × light/dark × phone/desktop (12 projects, one worker). Small per-PR manifests must be expanded before the milestone. Performance scenarios retain their specified project restrictions and run exclusively. A matrix denominator counts executed test configurations, not merely configured projects; skipped and unexecuted checks remain limited.

## Shared budgets and validity

SPEC §23.8: shell and each lazy route closure ≤60 KiB gzip; precache ≤600 KiB gzip; CLS 0; no scroll/navigation task >50 ms; own-input work ≤8 ms p95; cached warm content <1 s. SPEC §23.12: frame work ≤4 ms p95/≤8 ms max, no Chromium long tasks; 120 Hz frame/input cadence unproven on a 60 Hz host. WebKit script/layout/paint attribution is required before explaining jank as headless. R-LIVE is native effect to visible replica record ≤1 s p95; a browser sim live check does not certify it. Preserve timing endpoints, sample counts, p50/p95/max, load/swap and causal attribution. No threshold changes.

## App gates

- Reminders: WEB-REM-READ, DETAILS, WRITE, ENTRY, PARITY; WRITE also real. Read-model scenarios WEB-RM-REMINDERS-* at sim; REM-STORE-READ sim/real; REM-STORE-DECODE sim; REM-STORE-PERF sim/real. Gateway 1,000-item items <10 ms p95. Store 200-sample changed 1/10 <5 ms p95, changed 100/1,000 and unchanged <50 ms, full warm <100 ms, fresh full main-queue blocking <16 ms; cold separately. R-LIVE CORE-LIVE-LATENCY real (Reminders source), plus REM-CREATE-EXTERNAL real. Real enrichment attribution requires interleaved A/B/A/B and restart if initial swap >7 GB. Automation parity stays hidden while unproven; store-only flag propagation and missing positive fixtures remain limited.
- Calendar: WEB-CAL-READ and PARITY, CAL-STORE-READ sim/real/web; WRITE planned until merged. Month/1,000-event calendar.range ≤10 ms p95 REST/MCP. R-LIVE CAL-CREATE-EXTERNAL real. Invitation replies/travel/attachment actions stay hidden without verified capabilities.
- Contacts: WEB-CON-READ/WRITE/PARITY (read web, write/parity web+real); WEB-CON-LIST and CON-GROUPS sim. R-LIVE CON-EXTERNAL real. No separate per-app numeric read-model budget specified; shared UI budgets still apply. Wait for owning units; do not treat missing scenarios as pass.
- Messages: all WEB-MSG-* and associated WEB-RM-* browser scenarios in features.yaml; writing web/real as declared. Exclusive WEB-PERF-SHELL, LIST, SEARCH, WEB-MSG-PERF, LIVE and WEB-LIVE-DELTA. WEB-RM-CHATS first page at 500 chats ≤15 ms p95 REST/MCP; 100-item thread page on a 170k-message store and originator ≤10 ms; demo startup <10 s; bounded response-to-DOM search ≤50 ms; sim inbound browser ≤500 ms p95. R-LIVE uses owned self-handle fixtures only; rich field Truth and read-only real UI are separate evidence. Known main chats/startup misses and unproven frame/input/WebKit attribution remain limited until measured or fixed. Huge fixture setup/originator improvements after #191 require fresh verification.
- Notes: NOTE-DECODE-FULL sim/real; WEB-NOTE-READ web, WRITE/PARITY web+real. R-LIVE NOTE-EXTERNAL real. No separate numeric app budget specified; shared UI budgets apply. Units absent on this inventory SHA, so the gate is not ready.

## Scenario manifest inventory

### Reminders

- `WEB-REM-DETAILS`: [web]; `lab/scenarios/web/WEB-REM-DETAILS.yaml`.
- `WEB-REM-ENTRY`: [web]; `lab/scenarios/web/WEB-REM-ENTRY.yaml`.
- `WEB-REM-PARITY`: [web]; `lab/scenarios/web/WEB-REM-PARITY.yaml`.
- `WEB-REM-READ`: [web]; `lab/scenarios/web/WEB-REM-READ.yaml`.
- `WEB-REM-WRITE`: [web, real]; `lab/scenarios/web/WEB-REM-WRITE.yaml`.

### Calendar

- `WEB-CAL-PARITY`: [web]; `lab/scenarios/web/WEB-CAL-PARITY.yaml`.
- `WEB-CAL-READ`: [web]; `lab/scenarios/web/WEB-CAL-READ.yaml`.

### Contacts

- `WEB-CON-LIST`: [sim]; `lab/scenarios/web/WEB-CON-LIST.yaml`.
- `WEB-CON-PARITY`: planned; no manifest on inventory SHA.
- `WEB-CON-READ`: planned; no manifest on inventory SHA.
- `WEB-CON-WRITE`: planned; no manifest on inventory SHA.

### Messages

- `WEB-MSG-A11Y`: planned; no manifest on inventory SHA.
- `WEB-MSG-ATTACH-SEND`: planned; no manifest on inventory SHA.
- `WEB-MSG-AUDIO`: [web]; `lab/scenarios/web/WEB-MSG-AUDIO.yaml`.
- `WEB-MSG-AVATAR-READY`: [web]; `lab/scenarios/web/WEB-MSG-AVATAR-READY.yaml`.
- `WEB-MSG-CARD-SURFACES`: [web]; `lab/scenarios/web/WEB-MSG-CARD-SURFACES.yaml`.
- `WEB-MSG-CHAT-ACTIONS`: planned; no manifest on inventory SHA.
- `WEB-MSG-CHATS-CACHE`: [web]; `lab/scenarios/web/WEB-MSG-CHATS-CACHE.yaml`.
- `WEB-MSG-CHATS-STORE`: [web]; `lab/scenarios/web/WEB-MSG-CHATS-STORE.yaml`.
- `WEB-MSG-COMPOSER`: [web]; `lab/scenarios/web/WEB-MSG-COMPOSER.yaml`.
- `WEB-MSG-COMPOSITION`: [web]; `lab/scenarios/web/WEB-MSG-COMPOSITION.yaml`.
- `WEB-MSG-DETAILS`: [web]; `lab/scenarios/web/WEB-MSG-DETAILS.yaml`.
- `WEB-MSG-EDIT`: [web]; `lab/scenarios/web/WEB-MSG-EDIT.yaml`.
- `WEB-MSG-EDIT-UNSEND-VIEW`: planned; no manifest on inventory SHA.
- `WEB-MSG-EFFECTS`: [web]; `lab/scenarios/web/WEB-MSG-EFFECTS.yaml`.
- `WEB-MSG-EFFECTS-TRANSCRIPT`: [web]; `lab/scenarios/web/WEB-MSG-EFFECTS-TRANSCRIPT.yaml`.
- `WEB-MSG-FAIL-RESEND`: planned; no manifest on inventory SHA.
- `WEB-MSG-FILES`: [web]; `lab/scenarios/web/WEB-MSG-FILES.yaml`.
- `WEB-MSG-FILTERS`: [web]; `lab/scenarios/web/WEB-MSG-FILTERS.yaml`.
- `WEB-MSG-FILTERS-MENU`: [web]; `lab/scenarios/web/WEB-MSG-FILTERS-MENU.yaml`.
- `WEB-MSG-FONTS`: [web]; `lab/scenarios/web/WEB-MSG-FONTS.yaml`.
- `WEB-MSG-GROUP-EVENTS`: [web]; `lab/scenarios/web/WEB-MSG-GROUP-EVENTS.yaml`.
- `WEB-MSG-INTERACTION-MOTION`: [web]; `lab/scenarios/web/WEB-MSG-INTERACTION-MOTION.yaml`.
- `WEB-MSG-KEYBOARD`: [web]; `lab/scenarios/web/WEB-MSG-KEYBOARD.yaml`.
- `WEB-MSG-KEYBOARD-LIST`: [web]; `lab/scenarios/web/WEB-MSG-KEYBOARD-LIST.yaml`.
- `WEB-MSG-LINKS`: [web]; `lab/scenarios/web/WEB-MSG-LINKS.yaml`.
- `WEB-MSG-LIST`: [web]; `lab/scenarios/web/WEB-MSG-LIST.yaml`.
- `WEB-MSG-LIVE`: planned; no manifest on inventory SHA.
- `WEB-MSG-LOADING`: [web]; `lab/scenarios/web/WEB-MSG-LOADING.yaml`.
- `WEB-MSG-MEDIA`: [web]; `lab/scenarios/web/WEB-MSG-MEDIA.yaml`.
- `WEB-MSG-MOTION`: [web]; `lab/scenarios/web/WEB-MSG-MOTION.yaml`.
- `WEB-MSG-NEW`: [web]; `lab/scenarios/web/WEB-MSG-NEW.yaml`.
- `WEB-MSG-OPTIMISTIC`: [web]; `lab/scenarios/web/WEB-MSG-OPTIMISTIC.yaml`.
- `WEB-MSG-PARITY-UI`: planned; no manifest on inventory SHA.
- `WEB-MSG-PARTS`: [web]; `lab/scenarios/web/WEB-MSG-PARTS.yaml`.
- `WEB-MSG-PENDING`: [web]; `lab/scenarios/web/WEB-MSG-PENDING.yaml`.
- `WEB-MSG-PERF`: planned; no manifest on inventory SHA.
- `WEB-MSG-PLACEHOLDERS`: [web]; `lab/scenarios/web/WEB-MSG-PLACEHOLDERS.yaml`.
- `WEB-MSG-POINTER`: [web]; `lab/scenarios/web/WEB-MSG-POINTER.yaml`.
- `WEB-MSG-REACT`: [web]; `lab/scenarios/web/WEB-MSG-REACT.yaml`.
- `WEB-MSG-REACTIONS`: [web]; `lab/scenarios/web/WEB-MSG-REACTIONS.yaml`.
- `WEB-MSG-REAL-READ`: planned; no manifest on inventory SHA.
- `WEB-MSG-REORDER`: [web]; `lab/scenarios/web/WEB-MSG-REORDER.yaml`.
- `WEB-MSG-REPLIES`: [web]; `lab/scenarios/web/WEB-MSG-REPLIES.yaml`.
- `WEB-MSG-REPLY`: [web]; `lab/scenarios/web/WEB-MSG-REPLY.yaml`.
- `WEB-MSG-REPLY-HANDOFF`: [web]; `lab/scenarios/web/WEB-MSG-REPLY-HANDOFF.yaml`.
- `WEB-MSG-RICH-PARTS`: [web]; `lab/scenarios/web/WEB-MSG-RICH-PARTS.yaml`.
- `WEB-MSG-SEARCH`: [web]; `lab/scenarios/web/WEB-MSG-SEARCH.yaml`.
- `WEB-MSG-SEARCH-LIST`: [web]; `lab/scenarios/web/WEB-MSG-SEARCH-LIST.yaml`.
- `WEB-MSG-SEND`: planned; no manifest on inventory SHA.
- `WEB-MSG-SIDEBAR`: [web]; `lab/scenarios/web/WEB-MSG-SIDEBAR.yaml`.
- `WEB-MSG-STICKERS-GENMOJI`: [web]; `lab/scenarios/web/WEB-MSG-STICKERS-GENMOJI.yaml`.
- `WEB-MSG-TEXT-SIZING`: [web]; `lab/scenarios/web/WEB-MSG-TEXT-SIZING.yaml`.
- `WEB-MSG-THREAD-CORE`: [web]; `lab/scenarios/web/WEB-MSG-THREAD-CORE.yaml`.
- `WEB-MSG-TRANSCRIPT-HOOKS`: [web]; `lab/scenarios/web/WEB-MSG-TRANSCRIPT-HOOKS.yaml`.
- `WEB-MSG-TRANSCRIPT-VIEWER`: [web]; `lab/scenarios/web/WEB-MSG-TRANSCRIPT-VIEWER.yaml`.
- `WEB-MSG-UNSEND`: [web]; `lab/scenarios/web/WEB-MSG-UNSEND.yaml`.
- `WEB-MSG-VIEWER`: [web]; `lab/scenarios/web/WEB-MSG-VIEWER.yaml`.

### Notes

- `WEB-NOTE-PARITY`: planned; no manifest on inventory SHA.
- `WEB-NOTE-READ`: planned; no manifest on inventory SHA.
- `WEB-NOTE-WRITE`: planned; no manifest on inventory SHA.

## Matrix tooling dry-run

Air command: `heavy.sh --light bash -c 'cd web && pnpm install --frozen-lockfile && pnpm exec playwright test --list -g "WEB-(REM|CAL|CON|NOTE)-"'`: PASS, 192 configurations across 12 projects on the initial inventory tree. Reminders owns 120 configurations (read 24, details 12, entry 24, writes 48, hidden parity 12). The write draft-race test originally used an unregistered `WEB-REM-WRITE-FOCUS` title; the runner’s exact-ID matcher excluded it from every registered gate. It now shares `WEB-REM-WRITE`, preserving all assertions and including it in that scenario’s 48 configurations. No Contacts or Notes UI tests exist on the inventory SHA.

## Readiness correction

The leader subsequently identified final units still in flight: Reminders selection/columns #234 and Contacts group UI #236. The initial Reminders read (24/24) and details (12/12) checks on 2303b82a are archived privately under `lab/evidence/private/milestone/pre-readiness-2303b82a/`; they are pre-readiness checks, not the completed app milestone. The old-revision batch was stopped before writes/performance/real effects. Start the first app whose final unit merges and refresh the inventory and build at that revision. The leader reports #235 fixes chats p95, demo startup and the 170k thread; those reported results still require fresh milestone verification.

## Contacts gate revision

Refreshed to 926349b5 after #236 merged. Contacts milestone manifests target all 12 projects: READ (12 configurations), WRITE (36), PARITY (12). READ passes 12/12 and saves private list/card captures for each project. The first phone WRITE and PARITY runs exposed missing Lists navigation in the test journeys. Firefox then exposed a real capability-response race: inserting New Contact moved Select from y=64 to y=120 during a click. The repair reserves the hidden, disabled button’s layout space until capability proof arrives. A controlled real-response barrier reproduces the original shift and proves the repaired geometry, retaining all original assertions and timeouts. Fresh READ (12/12), WRITE (36/36) and PARITY (12/12) matrices pass without skips or flakes. The exclusive shared performance gate fails the combined Fixtures route closure (71,037 bytes gzip versus 61,440); fixture-route splitting follows #237. Contacts-specific own-input, layout/paint and warm-content timings remain unproven by that shared Messages/foundation probe. Native CON-EXTERNAL passes 20 external changes at each endpoint: gateway p50/p95/max 415.9/795.7/837.3 ms and replica 447.9/807.8/880.0 ms. CON-CRUD verifies six REST/MCP saves and cleanup, but native command visibility misses the 1-second budget: gateway p95/max 1174.3 ms, replica 1210.2 ms. CPU samples attribute the window to Helper/gateway/contactsd work, so this is a product miss. CON-IMAGE passes Truth-installed image read/digest proof. Native photo-command fidelity, 200-change/5-Hz burst acceptance and kill-during-apply qualification remain limited; these checks do not prove them. Native follow-up uses the existing guarded CON-EXTERNAL, CON-CRUD and CON-IMAGE real scenarios. Native photo command fidelity remains a separate claim from reading an image installed by Truth.

## Canonical subflow discovery

Unregistered hyphenated subflow titles now use whitespace after their longest registered owning scenario ID. Registered child scenarios retain their own IDs and fixture selection. `make features` enumerates Playwright’s expanded titles and fails any scenario-prefixed title whose complete leading ID has no manifest. The check applies across all apps and performance titles. Its first run caught two additional generated command-part titles missed by a source-only scan. All 57 edited title declarations retain their test bodies and assertions; newly included scenarios still require execution and any unavailable platform coverage must be recorded as limited.
