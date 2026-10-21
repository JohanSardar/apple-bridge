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

**R (read):**
- A Core Data store reader in the Helper: consistent backup copy, same pattern as Notes (§9.5).
- Fields: flags, parent/subtasks, sections and membership, tags, list icon/emoji/color/group/order, smart lists (definitions), manual order hints, images (blobs plus variants), due-date delta alerts, assignments.
- EventKit stays the write path and the identity anchor. Store rows link to EventKit identifiers through the reminder's `calendarItemIdentifier` / external id.
- AppleTruth reads the store independently.
- Read models: `reminders.lists` (sidebar with counts), `reminders.items{list|smart|tag, show_completed, sort}`.

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
