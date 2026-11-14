# Remove Safari, Shortcuts and Mac settings

The owner asked on 2026-11-14 to scrap Safari, Shortcuts and Mac settings and to turn off `make sim` for now. No agent, CLI command, MCP tool, REST route or document offers them any more:

- Safari: history, bookmarks, tabs and Reading List.
- Shortcuts: listing installed shortcuts and running one (`shortcuts.run`, `shortcuts.actions`).
- Mac settings: battery, display, network, Bluetooth, Time Machine and song recognition reads, and volume, appearance, apps, clipboard, notifications, power and wallpaper changes (every `mac.*` operation except `mac.screen_record`).

The catalog drops from 161 to 138 operations and from 46 to 33 collections. The `mac`, `safari` and `shortcuts` scopes, their domains, readers, Swift targets, permission names (`bluetooth`, `notifications`, `automation_safari`, `automation_shortcuts_events`), lab scenarios and feature entries go with them.

Two things stay because other sources depend on them:

- `mac.screen_record` keeps its name but belongs to Capture: scope `capture:write`, source `capture`. An agent that already holds `capture:write` (for screenshots, photos or audio) can now record the screen without a new grant; approve-level agents still ask first.
- The signed Shortcuts pack runner (`ab-shortcuts`, feature `core.shortcuts`, `bridge shortcuts install`) stays, because Focus and Clock change the Mac through the bundled AB Focus and clock shortcuts. It runs only those bundled shortcuts; agents cannot run a shortcut by name.

An upgraded home keeps what it already stored, but nothing serves it: records of the removed collections are retired like any unknown source, old ledger commands for them are skipped instead of stopping startup (reusing one of their ids is refused as a reused id, and one left waiting keeps its last published state), stored grants such as `safari:read` are dropped when credentials load (and are gone from `credentials.json` after its next write, so a downgrade does not get them back), an OAuth refresh or replica welcome that still names them is accepted without them, and capture overrides that name a removed source are ignored. `make sim` prints that it is turned off and runs nothing; `packaging/dev/rust-lab.sh sim` still runs named scenarios.
