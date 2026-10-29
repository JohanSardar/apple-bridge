# Screen Time probe on Johan's MacBook Air (2026-10-16, real test)

Air: macOS 26.7.1 (25G241), SIP enabled. Accessed over SSH from the Ubuntu laptop. The session has Full Disk Access: knowledgeC and Biome "restricted" streams are readable.

| Source | Result |
|---|---|
| ScreenTimeAgent store (RMAdminStore, DeviceActivity summaries) | **Operation not permitted** even with FDA. Vault confirmed. |
| knowledgeC `/app/usage` | Readable. This Mac is current. Another Mac (the MacBook Pro) appears until Oct 4. No iPhone rows. |
| Biome `App.InFocus` local | Readable, current. The Air's own app focus. |
| Biome `App.InFocus` remote | **Readable.** Device A9937F8D… = the iPhone (platform 2, build 24A446). Device 3D0A0ADB… = the MacBook Pro (platform 3, build 25F84). Device names come from `~/Library/Biome/sync/sync.db` table `DevicePeer`. |
| Biome `ScreenTime.AppUsage` | Local stream only; not decoded yet. |

Decoder: `../probe/infocus_usage.py` (SEGB records: field 3 = starting flag, field 4 = Mac-epoch double timestamp, field 6 = bundle id).

Example output: per-app minutes per device per day for the iPhone and both Macs (personal values not committed).

Known gaps to fix:
- Mac totals are inflated when the screen is locked or the display is asleep. The app stays "in focus" (Air: Chrome 13 h; Pro: `loginwindow` = lock screen). Clip sessions using the display/lock Biome streams.
- iPhone data reaches the Mac in batches. At 21:41 the newest iPhone event was 01:29, about 20 h behind, so this is not real time.
- Not yet compared with the totals in the Screen Time settings screen.

## T52 action comparison — 2026-10-28, real inventory probe

Decision: **reject replacement for now; retain Biome**. Action output availability and parity are unqualified, rather than established as unavailable on macOS.

| Coverage / count | Biome App.InFocus | Get App & Website Data |
|---|---|---|
| Local filesystem coverage | 2 immediate local entries observed | Unmeasured |
| Remote filesystem coverage | 3 immediate remote entries observed | Unmeasured |
| App/day and device rows | Existing source decodes app focus and clips lock/sleep; not recounted by this inventory probe | Unmeasured |
| Website coverage | No website usage source | Unmeasured |
| Dedicated imported probe | No shortcut required | `AB Screen Time` absent among 14 installed shortcuts |

`USE-ACTION-PROBE` inspected shortcut names in memory and counted only immediate filesystem entries. Entry counts are not event, device, app or usage counts. No shortcut was executed and no durations, app names, website names or device identifiers were retained. The earlier Biome decoding evidence above remains the baseline; this run does not establish data freshness or numeric parity.

**Limited:** a dedicated qualified pack is missing. Setup path: [`packaging/shortcuts/screen-time-probe.md`](../packaging/shortcuts/screen-time-probe.md); intended owner export: `packaging/shortcuts/AB Screen Time.shortcut` (not present). Johan must create/import and approve this dedicated pack before a bounded output probe can compare matching date windows, app/device/website row counts and output field coverage. An installed shortcut name alone never authorizes execution.
