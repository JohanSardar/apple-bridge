# Calendar (web) spec

Status: W6 draft, waiting for Johan's skim. The format follows `messages.md` and `reminders.md`. **When** within W6 is R (read), W (write), P (parity) or hide.

**Bridge facts:**
- EventKit window is 2 years back to 2 years ahead, sliding daily.
- Events carry title, notes, location, url, start, end, all-day, time zone, recurrence (on every occurrence), exception flag, attendees (read-only), alarms, availability, status and calendar.
- Calendars carry title, account, color, writable and type.
- Writes: create, update and delete with span `this|future` (`all` rejects), and create calendar.
- EventKit doesn't expose travel time, attachments, invitation replies (accept/decline), attendee editing or a structured location (geo).
- `Calendar.sqlitedb` in the Calendar group container holds most of these. Read parity can come from it the same way as Reminders.

## 1. Layouts and views

| Width | Layout | Mirrors |
|---|---|---|
| ≥ 1024 px | Toolbar (Day / Week / Month / Year segmented control, Today, ‹ ›, search, +), optional calendars sidebar (accounts → calendars with color checkboxes, mini month), main view, event inspector popover | macOS Calendar |
| < 1024 px | Month grid with an event list below (iOS default), or Day / List; tap an event to push details; "Calendars" sheet; "+" | iPhone Calendar |

| View | Spec |
|---|---|
| Day | All-day strip on top, then a 24-hour timeline with a now-line (red dot and line, updated each minute) and overlapping events laid out in columns. Desktop shows the mini month and event details beside it |
| Week | 7 columns, sticky all-day row, the same timeline layout, and today's column highlighted |
| Month | 6×7 grid. Desktop shows event chips (colored dot + title + time) with "+N more"; phone shows dots per day plus the selected day's list |
| Year | 12 mini months; desktop heat-tint optional; tapping a day opens it |
| List (phone) | Chronological agenda with sticky day headers |

**Navigation:**
- Swipe or trackpad moves between periods with continuous scroll: vertical months on phone, horizontal on desktop with momentum.
- Pinch zooms the hour height in Day/Week.
- Keys: ⌘1–⌘4 for Day/Week/Month/Year, ⌘T for Today, ⌘←/⌘→ for previous/next.

**Deep links:** `/calendar/<view>/<yyyy-mm-dd>` and `/calendar/event/<id>`.

## 2. Events

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Event block | Calendar color (tinted fill + solid left bar), title, time, location; declined events struck through; tentative ones hatched | Same | R; status R | R |
| All-day / multi-day | Spans across days in the all-day row and month grid | Same | R | R |
| Recurring | Repeat glyph; the edit/delete prompt asks "This Event Only / All Future Events" | Same two choices (no "All") | R; W with span this/future | R / W |
| Create | Drag on the timeline, double-click, "+", or quick event with natural language ("Lunch with Sam Friday 1pm") | Same; natural language parsed client-side | create W | W |
| Move / resize | Drag the block or its edges with 15-min snapping and live time labels | Same, with springy snap | update W | W |
| Details inspector | Title, location (map snapshot, Get Directions link), all-day, starts, ends, time zone, repeat, travel time, calendar, invitees, alert(s), show as, URL, notes, attachments | Every field EventKit carries; map snapshot from the location text via a maps link (no geo) | R/W; travel time, attachments and geo `—` | R / W / P |
| Invitees | List with response status; organizer | Read-only list with status badges | R (attendees) | R |
| Respond to invitation | Accept / Maybe / Decline | Hidden unless provable through Accessibility | `—` | P (investigate) / hide |
| Alerts | None, At time, 5/10/15/30 min, 1/2 h, 1/2 days, Custom; second alert | Same | R/W | W |
| Time zone override | Per-event time zone | Same | R/W | W |
| Delete | Confirm, with span for recurring events | Same | W | W |
| Search | Across titles, locations, notes and invitees | Same, results grouped by date | R (`search`) | R |
| Birthdays / holidays | Read-only subscribed calendars | Shown, read-only | R (calendar type) | R |
| Calendars | Show/hide per calendar (local preference), color | Same; hiding is per device | R | R |
| New calendar | Name, color, account | Same | create_calendar W | W |
| Conflicts | Overlap layout | Columns layout algorithm with tests (layout math) | — | R |

## 3. Motion

| Moment | Spec |
|---|---|
| Period change | Pages slide with the finger and settle by velocity (springs). Month → day zooms from the tapped cell |
| Create by drag | Ghost block grows under the pointer with live time labels; release springs to snapped times |
| Move / resize | Lift (shadow), track 1:1, drop with a spring to the snapped position |
| Now-line | Moves smoothly every minute (CSS transform) |

## 4. Bridge work

**R:**
- Read models `calendar.range{start, end, calendars[]}`. It returns events laid out in time order with the overlap groups precomputed, kept under 10 ms p95 for a month.
- Store-backed fields: travel time, attachments as blobs, structured location, invitation status of the owner.
- AppleTruth checks.

**Store mapping (R).** `Calendar.sqlitedb` lives in the Calendar group container. Column names were read on the Air (macOS 26.7.1), schema only.

| Field | Store source |
|---|---|
| link to EventKit | `CalendarItem.unique_identifier` / `UUID` / `external_id` (the tagged macOS 26.7.1 (25G241) fixture proves `UUID` equals EventKit's `calendarItemIdentifier`, `unique_identifier` equals its external identifier, and `Calendar.UUID` equals its calendar identifier) |
| travel time | `CalendarItem.travel_time`, `travel_advisory_behavior`, start location via `start_location_id` → `Location` |
| structured location | `Location.title`, `address`, `latitude`, `longitude`, `radius`, `mapkit_handle` (via `location_id`) |
| attachments | `Attachment(owner_id → CalendarItem.ROWID, file_id → AttachmentFile)`; bytes become blobs with variants |
| owner's invitation status | `CalendarItem.invitation_status`, `self_attendee_id` → `Participant.status` |
| attendee proposals and comments | `Participant.proposed_start_date`, `comment` (display only) |
| conference / video link | `Conference` table (display as a join link) |
| privacy, priority | `CalendarItem.privacy_level`, `priority` |
| alarms (extra detail) | `Alarm.trigger_interval`, `trigger_date`, `proximity`, `location_id`, `default_alarm` |

Change detection reuses the existing Calendar database hints from `f585a30`, with bounded coalescing and never one refetch per hint (see LIVE-FIX).

W6-STORE reads an in-memory SQLite backup opened read-only, and checks the required schema before joining. Joins must identify exactly one item in the EventKit calendar; detached occurrences are not inferred from their master. Each store field carries `store_coverage`, and an unavailable store or uncertain identity publishes nulls with an explicit reason. Positive travel-time units, coordinate units, start locations, attendee proposals/comments, extra store alarms and attachment derivatives remain limited until a positive native fixture proves their representation. The UI hides those unverified capabilities.

Locally available attachment originals become content-addressed blobs. Reads remain within the Calendar group container, reject dataless files, and authorize only registered opaque handles. No read hydrates an iCloud placeholder. AppleTruth independently counts only this run's tagged items and attachments and compares identity columns without publishing identifiers.

`calendar.range` uses half-open instant bounds and floating civil dates in the viewer zone, with calendar and text filters. Timed and all-day overlap groups are separate. A bounded 16-page/8 MiB cache holds computed layouts; its revision includes the durable Calendar record sequence and subscription epoch/revision. Visibility is checked before cache lookup, and every response carries the current transaction's cursor. The 1,000-event month scenario measures REST and MCP server computation against the 10 ms p95 budget; cached pages are copied before returning.

**W:** existing EventKit commands, plus the "This Event Only / All Future Events" prompt in the UI.

**P:** invitation responses, travel time and attachments are written through Calendar.app Accessibility only where provable. Otherwise they stay hidden.

## 5. Hidden

- Calendar sharing and publishing
- Delegates
- Subscribing to new calendars
- Apple Intelligence suggestions
- Holiday calendar management

## 6. Calendar read interface

The read interface exposes Day, Week, Month, Year and List through `/calendar/<view>/<civil-date>` and event inspectors through `/calendar/event/<record-id>`. It subscribes to `calendar.range`, and the inspector subscribes to its individual record. Trusted-device calendar visibility uses the existing preference store and is cleared when persistence is disabled; search stays in memory. Public sessions write no Calendar data to browser storage. Search covers the captured window around the displayed date.

The month uses civil dates, including on phones in a different zone from an event. Timelines render overlap columns from the read model, show floating all-day rows separately, and display a current-time line. The inspector displays timed bounds in the event's declared zone and floating bounds as inclusive visible dates. Unverified store fields and parity write actions stay hidden.

Arrow keys page periods while the content is focused, Command/Control Left/Right page globally, T or Command/Control T returns to today, Command/Control 1–4 choose Day through Year, and Command/Control F focuses search. Touch paging settles with velocity, and timeline zoom supports pinch or Control-wheel. Reduced motion removes the spring and now-line animation.
