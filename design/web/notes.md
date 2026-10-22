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

## 5. Captured rich content

`notes.note.data.blocks` uses the `NotesBlock` / `NotesRun` contract in `internal/contracts/notes_blocks.go`. Block types are `paragraph`, `title`, `heading`, `monostyled`, `list_item`, `checklist_item`, `quote`, `table`, `attachment` and `unknown`. Headings carry `level`, `collapsible` and nested `children`; lists carry `list_style` and indentation `level`. Checklists carry `checked` and the native `checklist_id`. Tables carry ordered `rows`, each containing cells with their own blocks. Inline runs preserve emphasis, links, highlights and attachment IDs.

Attachment IDs resolve through `data.attachments`: metadata, an optional original `blob`, and available `derivatives`. Blob handles and filesystem paths remain private. Missing bytes and unknown block attributes have explicit reasons. `data.html` is escaped fallback rendering, not a qualified write round trip. `block_status: partial` signals unsupported attributes or unresolved tables; an undecodable body retains a `raw_body` blob reference. Locked notes never decode or stage their body or attachments.

Bodies that exceed the existing metadata frame budget use `body_status: external` and `body_blob: {digest,size,state}`. The verified UTF-8 JSON blob contains exactly `{plain_text,html,blocks}`; fetch it through the authenticated blob endpoint for that record. Metadata and attachment references stay on the record. `plain_text` remains inline when it fits; otherwise `preview` supplies the list snippet and `search_status: metadata_only` identifies the search limitation. External bodies stay read-only pending write qualification. A failed staging attempt reports `body_status: unavailable`, not ready content. Attachment metadata that alone exceeds the frame budget is explicitly partial (`attachment_status: metadata_capacity`); no frame limit is raised.

`pinned` is optional when the store lacks the field. Tags come only from body-referenced hashtag attachments belonging to that note. Recently Deleted and its descendants remain readable and reject writes. `round_trip_status: unproven` identifies a structurally serializable subset; it grants no rich-edit capability. Native format and variant completeness are qualified at the Notes milestone; rich editing requires the separate per-OS parity proof.

## 6. HTML edit qualification

`NOTE-EDIT-PARITY` runs the signed Helper against two notes it creates with a unique `[ab-test …]` tag in the Bridge Test folder. It tries one simple-rich HTML replacement and one checkbox-state replacement, compares the native SQLite projections, and deletes only the tagged notes it created. The reusable Bridge Test container is retained. The lab probe and control command are absent from production builds.

On macOS 26.7.1 (25G241), neither edit is qualified. The simple-rich replacement did not reproduce the expected plain text and block tree; the decoder also reported unknown note and paragraph attributes, preventing a lossless guarantee. Standard HTML checkbox inputs produced zero native checklist items before and after replacement, so neither native identity nor state preservation could be established. Four public mutation calls (two creates and two replacements) completed, and tagged-note cleanup was verified. Rich editing and checklist toggling remain hidden; existing plain-text commands retain their `editable: plain` guard.

Run the per-OS proof with `packaging/dev/heavy.sh go run ./cmd/bridge-lab run lab/scenarios/notes/NOTE-EDIT-PARITY.yaml --level real --json`. A future candidate must also pass the production command path before any capability is exposed; this probe never enables one automatically.
