# Contacts (web) spec

Status: W7 draft, waiting for Johan's skim. The format follows `messages.md`. **When** within W7 is R, W, P or hide.

**Bridge facts:**
- Read: unified contacts with names (including phonetic and nickname), organization, job title, department, phones, emails, postal addresses, URLs, birthday, dates, social profiles, instant messages, relations, image and thumbnail (blobs), container and writable.
- `note` is unavailable without Apple's restricted entitlement.
- Groups and membership are readable; group writes require the OS-build and default-container capability proof.
- Writes: create, update (field patch) and delete.
- Lookup by name, phone or email (`contacts.lookup`).

## 1. Layouts

| Width | Layout | Mirrors |
|---|---|---|
| ≥ 1024 px | Sidebar (All Contacts, accounts, lists/groups), contact list (search, A–Z sections, index scrubber), card | macOS Contacts |
| < 1024 px | Lists → contacts (large title, search, A–Z index on the right edge, "My Card" at top) → card | iPhone Contacts |

Deep links: `/contacts`, `/contacts/<id>`, `/contacts/list/<group>`.

## 2. List

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Sections | A–Z by the sort order (last or first name per preference), "#" for others | Same; the sort and display order preferences are per device | R | R |
| Index scrubber | Letter rail; drag scrubs with a haptic tick | Same; the drag shows a floating letter bubble | — | R |
| My Card | Top row with "My Card" | Same, if the "me" contact is identifiable | R (investigate "me" via the Contacts API) | R |
| Search | Name, phone, email, organization, address | Same, results as you type | R (`contacts.lookup`, `search`) | R |
| Row | Name, with company in secondary text for company contacts | Same | R | R |
| Groups / lists | Sidebar lists, counts | Same, read-only | R | R |
| Multi-select | Delete, export | Delete (confirm), export vCard | W delete | W |

## 3. Card

| Feature | Apple | Web | Bridge | When |
|---|---|---|---|---|
| Header | Large photo or monogram, name, phonetic, company | Same; the iOS 17+ Contact Poster is shown as the header image if readable, otherwise the photo | R image; poster `—` | R / P |
| Quick actions | message, call, video, mail | message opens our Messages new-message; call, video and mail open `tel:` / `facetime:` / `mailto:` on the viewer's device | — | R |
| Fields | Labeled phones, emails, addresses (with map snapshot link), URLs, birthday, dates, related names, social profiles, IM | Same order and labels | R | R |
| Notes | Notes field | Hidden (unavailable without the entitlement) | `—` | hide |
| Linked conversations | "Messages" history link | Shows recent chats with this person (via `messages.handle.contact_id`) | R | R |
| Edit | Inline edit mode: add/remove fields, label pickers, photo | Same; adding a photo uploads an image blob | update W (photo: verify) | W |
| Create | "+" → blank card | Same | create W | W |
| Delete | Bottom of edit mode, confirm | Same | delete W | W |
| Share | vCard | Download a `.vcf` generated client-side | R | R |
| Add to list | Group membership | Hidden until group writes exist | `—` | P |
| Merge duplicates | Card suggestions | Hidden | — | hide |

## 4. Motion

- Card push uses the pane transition. Row and card avatars remain contained and use the available captured photo variant.
- Edit mode morphs fields into their editable form in place.
- Index scrubbing gives instant jumps with a letter bubble.

## 5. Bridge work

**R:**
- Read model `contacts.list{group?, sort}` with section keys.
- Photo variants (thumb/display) through the media pipeline.
- "Me" identification.

**W:** existing commands, plus image set/remove if not already covered (verify).

**P:**
- Group membership writes (CNSaveRequest supports group add/remove; it was outside v1 scope).
- Contact poster read if it's stored accessibly.

## 6. Contacts list contract

`contacts.list` reads one local transaction and returns `sections`, `groups` (records with visible member counts), `containers`, `count` and the replication `cursor`. An optional `group` is a Bridge group ID; a missing or deleted group is `not_found`. Tombstoned contacts and stale membership IDs never count. Both REST and MCP expose the same shape, and the web live view replaces this snapshot after relevant commits.

`sort` and `display_order` independently accept `given` or `family`. Their defaults are `family` and `given`; the UI stores overrides on the viewing device. Company contacts use their organization; nameless contacts fall back to nickname/organization and ultimately “No Name”. Sorting folds case and combining accents, then uses the Bridge ID for ties. Sections are A–Z and `#` for other initials. This deterministic initial contract does not claim locale-specific collation.

Each item includes its complete contact record, display name, folded sort key, initials, nullable avatar reference, nullable `is_me`, and `conversation_ids`. Unknown identity stays null. Only explicit native identity may produce My Card; it is never inferred from names or handles. Linked chats use captured `messages.handle.contact_id` plus the participant index, are deduplicated, and disappear without Messages visibility. Contacts visibility is checked before records, counts, groups or containers are read.

Avatar references are enriched lazily; browser decode failures retain the monogram fallback.

The Contacts web route uses the shared live client, virtual list, app switcher and motion primitives. Viewer preferences are stored only for trusted sessions. Native identity uses `CNContactStore.unifiedMeContactWithKeys(toFetch:)` (unknown stays null), and account containers are captured alongside groups. Photo conversion uses the shared utility executor and media derivative cache, one Contacts job at a time, with cooperative foreground yields between stages; results enrich only contacts still citing the same original digest and generation. Capture and commands do not await photo conversion. The first available original or thumbnail remains a browser-decoded fallback, with a monogram on decode failure. Native identity, accounts and derivative behavior are pending the Contacts milestone's real qualification; no real-person or public-path proof is claimed by the UI PR. Notes, posters, membership editing and duplicate merging stay hidden.

## 7. Group command capability

`contacts.create_group{name}`, `contacts.rename_group{id,name}`, `contacts.delete_group{id}`,
`contacts.add_member{id,contact_id}` and `contacts.remove_member{id,contact_id}` use the public
`CNSaveRequest` route. Existing groups require `expected_version`; membership commands target
the group version. Only the default container is writable, and contacts must belong to that
same container. Removing membership or deleting a group never deletes its contacts.

Run the signed Helper with `selftest contacts-groups --report <private report path>` to qualify
this OS build and default container. The self-test creates its own tagged Bridge Test contact
and group, verifies every operation by a fresh public-framework read, and cleans both before
writing the private proof ledger. Failed or stale proofs leave controls hidden.

Contact posters remain **limited** and hidden: the public macOS 26.5 `CNContact.h` exposes
`imageData` and `thumbnailImageData`, with no poster property or fetch key. Ordinary contact
photos remain available. No private Contacts properties or database fields are used.

Current qualification is **limited**: on this Air, `removeMember` completed without the removal
becoming visible through fresh public Contacts reads within the bounded wait. The self-test
cleaned its tagged contact and group and withheld the ledger. Group controls remain hidden
until this is resolved and the complete proof passes.
## 8. Contact editing

Basic contact writes use the shared trusted-input command channel and captured versions. Inline edits preserve the starting version, so a concurrent change returns a conflict instead of replacing newer data. Review edit restores the draft against the current record; saving again is an explicit action. Optimistic cards resolve identities exclusively through `observed_record_ids` and settle when the live cursor reaches the observed commit. Uncertain delivery offers status lookup without redispatch.

The editor supports names, organization, labeled field arrays, dates and photo set/remove. Photos are decoded locally, resized to at most 512 pixels and encoded into the existing bounded image command; unsupported formats stay unsaved with an error. Notes remain hidden. Multi-selection permits up to 32 contacts for vCard export or individually versioned deletions after confirmation. Accounts conservatively marked unwritable have no edit/delete controls. Native write/photo fidelity, real Bridge Test cleanup and full browser qualification remain limited until the Contacts milestone; the focused PR checks use synthetic data.

The card's membership affordances use the shared trusted-input command layer, the current
group version and `observed_record_ids` with the captured cursor before settling. They are
restricted to writable contacts and groups in the same container and disappear when capability
or source readiness is absent. Group names and counts remain read-only in the web app; no
create, rename or delete list controls are exposed. Native membership controls remain hidden
while the complete group proof is limited. `WEB-CON-PARITY` exercises the conditional controls
against the isolated gateway fixture in Chromium light desktop; real-data acceptance remains
at the Contacts milestone.
