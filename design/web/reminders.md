# Reminders (web) spec

Status: W5 draft, waiting for Johan's skim (doesn't block earlier milestones). Format, sources and layout rules are the same as `messages.md`.

**Columns:** **Web** is what the browser does. **Bridge** is today's support: R read, W write, `—` missing. **When** is the step inside W5:
- **R:** read at full fidelity
- **W:** basic writes
- **P:** parity
- or **hide**

**Bridge facts:**
- Lists and reminders come through EventKit: title, notes, url, due, start, priority, completion, recurrence (next instance only) and alarms (including location).
- Writes: create, update (fields, move list), complete, uncomplete, delete, create list.
- EventKit has **no** flags, tags, subtasks, sections, list icons or groups, smart lists, manual order, images, assignments or early reminders.
- All of those live in Reminders' own Core Data store (`group.com.apple.reminders` … `Data-*.sqlite`), which the Helper can read with Full Disk Access, the same way it reads `chat.db`.
- Aggregate check on the Air: one store holds every reminder; flags, a subtask, lists and smart lists are all in use; no sections or tags yet.

## 1. Layouts

| Width | Layout | Mirrors |
|---|---|---|
| ≥ 1024 px | Sidebar (search, smart-list tiles in a 2-column grid, "My Lists" with groups, Tags), list content, details inspector (closed by default) | macOS Reminders |
| < 1024 px | Home: smart tiles grid + My Lists (+ groups) → push list → push details sheet | iPhone Reminders |

Deep links: `/reminders`, `/reminders/<list|smart:today|scheduled|all|flagged|completed|assigned|tag:<name>>`, `/reminders/<list>/<reminder>`.

## 2. Sidebar / home

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Smart tiles | Today, Scheduled, All, Flagged, Completed, Assigned to Me; each with a colored glyph and count; reorder or hide via Edit | Same, counts live | partial (computable from R fields; flags `—`) | R |
| Pinned lists | List-colored tiles with emblem and count after the smart tiles; omitted from My Lists | Same | pin, emblem and order R (store) | R (store) |
| My Lists | Colored circle with glyph or emoji, name, count; drag to reorder; groups as disclosure folders | Same | names and colors R; icon, emoji, groups and order `—` | R (store) |
| Custom smart lists | Tag/date/priority/flag filters | Shown and evaluated client-side | `—` | P |
| Tags section | `#tag` chips filter across lists | Same | `—` | R (store) |
| Add List | Name, color, glyph/emoji, type (standard/groceries), group | Name + color now; glyph, emoji and group later | create_list W | W / P |
| Search | Full-text across reminders | Same, with highlighted matches | R (`search`) | R |

## 3. List content

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Header | Large list-colored title, count of incomplete | Same | R | R |
| Row | Circle checkbox ringed in list color; title; notes (2 lines, secondary); due date/time (red when overdue); repeat glyph; location glyph; flag (orange); priority `!`/`!!`/`!!!`; tags; URL chip; image thumbnails; subtask count | Same | most R; flag, tags, images and subtasks `—` | R |
| Complete | Tap circle: fills with a spring; the row lingers ~1 s, then animates away (or stays when "Show Completed" is on) | Same, a **signature moment**: the ring fills with a radial spring, a check draws with a stroke animation, then the row folds away | complete/uncomplete W | W |
| Inline new | "New Reminder" row at the bottom; Enter creates the next one | Same, with natural-language dates ("tomorrow 9am", "every Monday") parsed client-side into fields | create W | W |
| Inline edit | Click title/notes to edit in place | Same | update W | W |
| Subtasks | Indented under the parent; drag onto another reminder to nest; ⌘] / ⌘[ | Same | `—` | R (store) / P |
| Sections | Headers inside a list; drag between; "Add Section" | Same | `—` | R (store) / P |
| Columns view | Kanban: sections as columns | Same (desktop) | `—` | P |
| Groceries list | Auto-categorized sections | Show sections as read | `—` | R / hide auto-categorize |
| Show Completed | Toggle; "Clear" completed | Same | R; delete W | R / W |
| Sort | Manual, Due Date, Creation Date, Priority, Title | Same; Manual needs order data | order `—` | R (store) |
| Drag reorder | Manual order | Same | `—` | P |
| Swipe (touch) | Delete; Flag; Details | Same | delete W, flag `—` | W / P |
| Multi-select | Shift/⌘-click; complete, delete, move, flag | Same | W | W |
| Assigned to | Shared lists: assignee avatar | Read-only avatar | `—` | R (store) |

## 4. Details inspector

| Field | Apple | Bridge | When |
|---|---|---|---|
| Title, notes, URL | Text fields | R/W | W |
| Date / time | Date picker with All-day toggle | R/W | W |
| Early reminder | e.g. 1 day before | `—` (due-date delta alerts) | R (store) / P |
| Repeat | Never, Hourly, Daily, Weekdays, Weekends, Weekly, Biweekly, Monthly, Every 3 Months, Every 6 Months, Yearly, Custom; End Repeat | R/W (next instance only) | W |
| Location | Arriving/Leaving + place + radius map | alarms R; W via EventKit location alarm | R / W |
| When Messaging | Person picker | `—` (vehicle/messaging trigger) | hide unless readable |
| Flag | Toggle | `—` | R (store) / P |
| Priority | None/Low/Medium/High | R/W | W |
| List | Move | W | W |
| Subtasks | Add/reorder | `—` | P |
| Images | Add photos | `—` | R (store) / P |
| Tags | Add `#tag` | `—` | R (store) / P |

## 5. Keyboard (mac-matching; check against the Air's menus during the capture pass)

| Keys | Action |
|---|---|
| ⌘N | New reminder |
| ⇧⌘N | New list |
| ⌘⏎ | Mark complete |
| ⌘I | Show info |
| ⇧⌘F | Flag |
| ⌘] / ⌘[ | Indent / outdent (subtask) |
| ⌘F | Search |
| ⌘1…⌘6 | Smart lists |
| ⌫ | Delete with confirm |

## 6. Motion

| Moment | Spec |
|---|---|
| Complete | Ring fill spring (response 0.3, damping 0.7), then check stroke over 180 ms, then row fold (height spring, 0.35/1.0) after a 900 ms linger that's cancelled if unticked |
| New row | The row grows in at the end with the caret already in the title |
| Reorder | Lifted row (scale 1.02, shadow); others make way with springs |
| Push / pop | Same as Messages |

## 7. Bridge work

The read UI uses `reminders.lists` and `reminders.items` views over `web/1`, with the viewer's IANA `time_zone`. Relevant Reminders record changes re-query the visibility-scoped read model and replace that view. Each view is capped at 2 MiB; capacity errors clear its rows and show an inline explanation, a known count when available, and guidance to choose a smaller list or tag. They never truncate the result. Cursor paging is the next W5 slice before writes if a counts-only real read proof finds any list or supported smart view above about 3,000 items; otherwise it stays a follow-up. Large notes or attachment metadata can reach the byte cap at smaller item counts.

Read navigation preserves the flat sidebar order and group disclosures, uses generic section headers and recursive subtasks, and offers completed visibility, server sorts, read-only details and search across titles, notes, URLs and tags. Search uses the bounded All projection with completed reminders included and highlights title and note matches. Details use a desktop inspector or a compact sheet. Phone push and back keep both panes rendered for the spring, then hide the outgoing pane; focus moves only after the incoming pane is shown. Reduced motion settles immediately. Live replacements preserve sidebar focus. Completed rows use a filled list-colored circle and secondary title text without strikethrough. The phone sidebar reserves space for its floating app switcher and the safe area. Review captures omit focus rings; keyboard captures retain them. Empty section headers retain a visible keyboard selection; Info and Enter open details only for reminder rows. Completion and other writes follow in W5-WRITE.

**R (read):**
- A Core Data store reader in the Helper: a long-lived read-only connection and short WAL snapshot transactions; cached typed statements and persistent-history incremental reads. The consistent-copy reader remains an independent oracle.
- Fields: flags, parent/subtasks, sections and membership, tags, list icon/emoji/color/group/order, smart lists (definitions), manual order hints, images (blobs plus variants), due-date delta alerts, assignments.
- EventKit stays the write path and the identity anchor. Store rows link by exact equality of the DA text identifier and normalized 16-byte UUID with EventKit's `calendarItemIdentifier`. External identifiers are never a fallback.
- AppleTruth reads the store independently.
- Read models: `reminders.lists` (sidebar with counts), `reminders.items{list|smart|tag, show_completed, sort}`.

**Store mapping (R).** Column names were read from the Air's store on macOS 26.7.1, schema only. Foreign-key column variants (`ZREMINDER1…5`, `ZLIST`) differ per entity; the W5-STORE worker verifies each against a tagged "Bridge Test" fixture.

| Field | Store source |
|---|---|
| reminder identity | `ZREMCDREMINDER.ZIDENTIFIER`; exact equality of normalized `ZIDENTIFIER` UUID bytes and `ZDACALENDARITEMUNIQUEIDENTIFIER` text with `calendarItemIdentifier`; never fall back to `ZEXTERNALIDENTIFIER` |
| flag | `ZREMCDREMINDER.ZFLAGGED` |
| subtask parent | `ZREMCDREMINDER.ZPARENTREMINDER` → `Z_PK` |
| list | `ZREMCDREMINDER.ZLIST` → `ZREMCDBASELIST` |
| manual order | list mergeable ordering column (excluded from the schema dump; locate it) and `ZICSDISPLAYORDER` |
| deleted / pending | `ZMARKEDFORDELETION` |
| early reminder | `ZDUEDATEDELTAALERTSDATA` (archived blob; decode with a pure fixture-tested decoder) |
| When Messaging | `ZCONTACTHANDLES` (blob; read-only, hidden unless decodable) |
| rich notes | `ZNOTESDOCUMENT` / `ZTITLEDOCUMENT` (prefer the EventKit plain text; decode only if needed) |
| list name, color, icon/emoji | `ZREMCDBASELIST.ZNAME`, `ZCOLOR`, `ZBADGEEMBLEM` |
| list groups | `ZISGROUP`, `ZPARENTLIST` |
| list order, pin | `ZDADISPLAYORDER`, `ZISPINNEDBYCURRENTUSER`, `ZPINNEDDATE` |
| sort style | `ZSORTINGSTYLE` |
| groceries | `ZSHOULDCATEGORIZEGROCERYITEMS`, `ZGROCERYLOCALEID` |
| shared list | `ZSHARINGSTATUS`, `ZSHAREDOWNERNAME` (display only) |
| smart lists | `ZREMCDBASELIST` rows with entity `REMCDSmartList`, `ZSMARTLISTTYPE`, `ZFILTERDATA` (blob) |
| sections | `ZREMCDBASESECTION.ZDISPLAYNAME`, `ZLIST`, `ZCREATIONDATE`; order from `ZSECTIONIDSORDERINGASDATA`; membership from `ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA` (blobs) |
| tags | `ZREMCDOBJECT` entity `REMCDHashtag` → `ZHASHTAGLABEL` → `ZREMCDHASHTAGLABEL.ZNAME` |
| images and files | `ZREMCDOBJECT` attachment entities (`ZUTI`, `ZFILENAME`, `ZFILESIZE`, `ZWIDTH`/`ZHEIGHT`, `ZSHA512SUM`), with bytes from the store's attachment files as blobs and variants |
| alarms | `ZREMCDOBJECT` alarm and trigger entities (date, time-interval, location `ZLATITUDE`/`ZLONGITUDE`/`ZRADIUS`/`ZPROXIMITY`) |
| assignment | `ZREMCDOBJECT` entity `REMCDAssignment` (`ZASSIGNEE`, `ZORIGINATOR`, `ZASSIGNEDDATE`) |
| recurrence | `ZREMCDOBJECT` entity `REMCDRecurrenceRule` (`ZFREQUENCY`, `ZINTERVAL`, `ZOCCURRENCECOUNT`, `ZENDDATE`, `ZFIRSTDAYOFTHEWEEK`) |

Change detection: enrichment refreshes after EventKit capture completion. There is no store watcher in this unit, so a store-only change depends on EventKit also posting a notification. The real flag-only propagation proof remains limited until the Helper has Reminders Apple Events authorization. If that proof finds a missing notification, follow up with a WAL FSEvents hint through the existing bounded coalescer, never one refetch per hint (see LIVE-FIX's AgendaGate findings).

**W (write):** existing EventKit commands, plus natural-language parsing in the client only.

**P (parity), via AppleScript where the dictionary supports it (flagged, verify per OS build), otherwise Accessibility on Reminders.app:**
- flag
- subtasks (indent/outdent)
- sections (create, move)
- tags
- list icon, emoji and group
- manual reorder
- images
- custom smart lists

Each gets a sim scenario plus a real scenario on a tagged "Bridge Test" list, with cleanup.

## 8. Hidden

- Grocery auto-categorization (writes)
- Templates
- Sharing and invitations
- When Messaging (unless readable)
- Apple Intelligence suggestions
