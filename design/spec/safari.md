# Safari capture

Safari is read-only in T41. Grants are safari:read and safari:write; no write capability is advertised. History covers 90 days, with visit counts within that window. Bookmarks retain folder paths; Reading List records expose added dates and whether DateLastViewed exists. Cloud tabs include device names, titles and URLs.

One source checkpoint generation resumes across sessions; per-database generations guard absence, with a second complete scan revalidating retained keys after replacement and history uses URL digest keys to prevent row ID reuse. History and cloud tabs use SourceSupport read-only SQLite snapshots and complete-scan guards. Unsupported schemas, truncation, replacement and malformed bookmarks do not publish absence. Only changed rows use the live lane after initial capture. A private cache stores native keys and database generations, never URLs or titles; writing a union before delivery preserves removal candidates across interrupted captures. Input signatures ignore SHM read marks and empty WAL sidecars so read-only snapshots cannot trigger a rescan loop. Each file has a WAL/file watcher with a 30-second fallback. Unchanged inputs skip complete scans; history also rescans when its oldest included visit expires from the rolling window. Failed scans remain eligible for retry. Full Disk Access is required. Missing CloudTabs databases degrade coverage independently.

Lab fixtures use synthetic SQLite databases and plists. Evidence contains counts, digests, booleans and CPU measurements only. Real Truth checks require independent read access to Safari files. SAF-IDLE measures five minutes of source process CPU in sim and the signed Helper process CPU in real.

CloudTabs table and column mapping matches the [mac_apt CloudTabs reader](https://github.com/ydkhatri/mac_apt/blob/master/plugins/safari.py#L243). This host has no CloudTabs.db, so real cloud-tab qualification remains unavailable.

Automation status is cached for at most 30 seconds during idle tab and readiness polling. Prepare and dispatch always refresh the non-prompting permission check. The five-second tab capture cadence and 30-second database fallback remain unchanged.
