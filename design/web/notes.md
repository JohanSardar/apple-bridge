# Notes (web) spec

Status: W8 draft, waiting for Johan's skim. The format follows `messages.md`. **When** within W8 is R, W, P or hide.

**Bridge facts:**
- Read: folders (account, parent) and notes (title, plain text, minimal HTML, created and modified time, locked flag, attachments list, content status, editable flag) from a consistent copy of `NoteStore.sqlite`.
- Locked notes are title-only.
- Writes through AppleScript: plain-text create, replace and append, delete, create folder.
- Notes with tables, checklists, drawings or attachments reject updates.

## 1. Layouts

| Width | Layout | Mirrors |
|---|---|---|
| ≥ 1024 px | Folders sidebar (accounts → folders, nested, Recently Deleted, Tags, Shared), note list (sorted, pinned section, previews), editor | macOS Notes |
| 700–1023 px | Note list + editor | iPad Notes |
| < 700 px | Folders → notes → editor | iPhone Notes |

Deep links: `/notes`, `/notes/folder/<id>`, `/notes/<id>`.

## 2. Folders and list

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Folders | Accounts with folders, nested disclosure, counts | Same | R | R |
| Smart folders / tags | Tag browser and smart folders | Shown if decoded; otherwise hidden | `—` | P (decode) |
| Pinned | Pinned section on top | Same | `—` (pinned flag in store; verify) | R |
| Row | Title (bold), date + first line (secondary), thumbnail of the first image, lock glyph, shared glyph | Same | R (attachments list; thumbnails need blobs) | R |
| Gallery view | Grid of note cards | Same (desktop) | R | R |
| Sort / group | Date edited, date created, title; grouped by date ("Today", "Previous 7 Days", …) | Same | R | R |
| Search | Full-text, including attachments' names | Same | R (`search`) | R |
| New note | ⌘N; the title comes from the first line | Same | create W | W |
| New folder | Name | Same | create_folder W | W |
| Delete / move | Swipe or menu; Recently Deleted for 30 days | Delete W; move `—`; Recently Deleted read-only | delete W | W / P |

## 3. Editor

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Rendering | Title, Heading, Subheading, Body, Monostyled, lists (bulleted, dashed, numbered), checklists (with checked state), tables, links, block quotes, highlights, collapsible sections, attachments (images, scans, PDFs, audio with transcripts, drawings as images), math results | Read-only rendering of everything the decoder produces, styled like Notes. Unknown blocks become a placeholder | R (minimal HTML; extend the decoder) | R |
| Editing plain-text notes | Rich editor | Plain-text editor for `editable: plain` notes with `expected_version`; autosave on trusted input (throttled keyup, navigation, Done/⌘S), never from timers, per §23.2 gesture guard | update W | W |
| Editing rich notes | Rich editor | Read-only with a banner explaining why; toggling checklist items is the first rich edit to investigate | `—` | P (investigate) |
| Locked notes | Password/Touch ID | Title-only row with lock glyph; content never shown | R (title only) | R |
| Attachments | Inline | Images via blob variants; PDFs as file chips; audio player | partial (list only; blobs `—`) | R |
| Share / collaborate | Share sheet | Hidden | — | hide |
| Quick Note, Math Notes | — | Hidden | — | hide |

## 4. Bridge work

**R:**
- Extend the Notes protobuf decoder to full block structure: styles, lists, checklists with state, tables, links, attachment references with blobs and variants, pinned, tags.
- Read model `notes.list{folder, sort}`.

**W:** existing plain-text commands, with autosave on trusted input (throttled keyup, navigation, Done/⌘S), never from timers, per §23.2 gesture guard. Dirty drafts warn on unload; conflicts keep the draft until explicitly reloaded.

**P (investigate, keep if provable):** checklist toggle and simple rich edits via AppleScript HTML bodies on notes the decoder can round-trip losslessly. Everything else stays read-only.
