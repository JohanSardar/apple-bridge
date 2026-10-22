# Messages (web) spec

Status: W0 draft, waiting for Johan's skim. Decisions: `design/DECISIONS.md` (2026-10-20, web app).

**How to skim:** each table row is one feature. **Web** says what the browser does. **Bridge** says what the bridge can do today (R read, W write, `—` missing). **When** gives the milestone, or **hide** or **placeholder**. Disagree with a row and it changes; everything else becomes the build checklist and the Playwright case list.

**Sources:**
- Apple's iOS 27 and macOS 27 UI kits (Design Resources, Sketch) for colors, type, materials and system components.
- An aggregate inventory of the Air's `chat.db`, counts only, to rank what actually appears in real conversations. Priority labels: **common**, **occasional**, **rare**.
- The bridge code (`native/Sources/MessagesSource`, `internal/contracts`).
- Captures of the real Messages app on the Air, filling §17 (pending: the Air's lid was closed during W0).

**"To verify"** marks behavior I'm not certain of until the capture pass.

**Reference version:** macOS 26.7 Messages on the Air for behavior. Where the 27 kits differ visually, the 27 kits win, because that's what new users see.

## 1. Layouts

| Width | Layout | Mirrors |
|---|---|---|
| ≥ 1024 px | Three panes. Sidebar (conversation list, 280–420 px, resizable): search field, pinned grid, list. Transcript in the middle. Details open as an inspector on the right (320 px), not a popover. | macOS Messages |
| 700–1023 px | Two panes. Sidebar plus transcript; details open as a sheet. | iPad Messages |
| < 700 px | Navigation stack: list, then transcript (push, swipe-back), then details (push). Large "Messages" title collapses into the bar on scroll. | iPhone Messages |

- The window title bar and toolbars use Liquid Glass "Regular" (§16). Lists and the transcript are solid.
- **Deep links:** `/messages`, `/messages/<chat_id>`, `/messages/<chat_id>/<message_id>` (scrolls to the message and highlights it), `/messages/new`. Back and forward and reload restore the view and scroll position.
- **PWA:** standalone display; safe areas respected, including the home indicator and the notch in landscape.

## 2. Conversation list

| Feature | Apple behavior | Web | Bridge | When |
|---|---|---|---|---|
| Row | Avatar 40 pt (iOS 52), name (Headline), one or two preview lines (Subheadline, secondary), time or date on the right (Footnote, secondary), chevron on iOS | Same. Time uses the viewer's locale and hour cycle (`Intl.DateTimeFormat`, so "9:41 AM" in en-US and "09:41" in en-GB), then "Yesterday", the weekday within 7 days, then the short date. Unnamed groups are titled from given names ("Alex, Sam & 3 more"; exact wording to verify in §17) | R | W2 |
| Avatar | Contact photo, else monogram on a gray gradient. Groups: up to 3 overlapping member avatars, or the group photo | Same. Monogram: first letter of the given name plus first letter of the family name, letters only; one letter when only one side is a letter; the person glyph when there is none (phone numbers, emails). Until the handles that determine the name resolve, show the neutral person glyph rather than provisional initials. Keep the glyph and initials in the same fixed avatar box; crossfade with opacity only for at most 150 ms, with no transition under reduced motion. A photo that fails or decodes empty falls back to the monogram. Group composite: 2 members on a diagonal, 3 in a triangle, each mini avatar ringed in the row background so overlaps read apart, monogram text about 0.42× its circle | R (contact images); group photo `—` | W2 / W4 |
| Unread | Blue dot left of the avatar; the preview is not bolded | Same, plus an unread count in the tab and app badge | R (`last_read_at`, `read_at`) | W2 |
| Preview text | Last message; attachments become "Attachment: 1 Image", "Video", "Audio Message", "Location", "Link"; reactions become "Loved 'text'"; system events are worded | Same wording table (§15) | R | W2 |
| Typing in row | Pins show a typing bubble (row behavior to verify) | Only if typing can be read (§13) | `—` | W4 / hide |
| Muted | Bell-slash icon after the name; no notifications | Same, shown only when the state is known to be muted; unknown shows nothing | `—` (Hide Alerts state) | W4 |
| Pinned grid | Up to 9 pins in a 3-column grid of large avatars. Unread pins show the latest incoming message in a bubble above the avatar (never a reaction removal); the bubble stays inside the safe area with its tail on the avatar. Drag to reorder; pin/unpin from the context menu or swipe | Same, including drag reorder with a spring | `—` (pins live in Messages prefs, iCloud-synced) | W4 read and write |
| Swipe actions (touch) | Leading: Mark read/unread. Trailing: Hide Alerts, Delete (full swipe deletes) | Same; full-swipe with a rubber band and haptic-style snap | Mark read W sim-only; Delete chat W sim-only | W3 |
| Context menu | Pin, Mark as Read/Unread, Hide Alerts, Delete; mac adds Open in New Window | Same, minus New Window; ⌘-click opens a new browser tab | as above | W3 / W4 |
| Filters | All Messages, Known Senders, Unknown Senders, Unread, Recently Deleted, Spam | Same menu, top-left on phone and in the sidebar header on desktop. Keep one stable all subscription, precompute All/Known/Unknown/Unread arrays from its rows, and start fixed deleted/spam views after paint. Switching filters creates no subscriptions. `known_sender` includes every participant, not only avatar handles, and requires no Contacts details; live deltas update sender and unread membership. All six client views equal the server-filter oracle | Unknown/Spam `—` (`chat.is_filtered`, `message.is_spam` not read); Recently Deleted `—` | W2 read, W4 actions |
| Recently Deleted | Deleted conversations and messages for 30 days; Recover, Delete permanently | Same view | `—` (recoverable tables not read) | W2 read, W4 write |
| Edit / select | Multi-select for Delete and Mark Read | Same; Shift/⌘-click ranges on desktop | as above | W3 |
| Compose button | Square-pencil, top right | Same; ⌘N. Hidden until W3 rather than shown disabled | — | W3 |
| Search field | Top of sidebar (mac); iOS placement to verify (bottom glass bar on 26+?) | Desktop: sidebar top. Phone: bottom glass bar (§8) | R (`search`) | W2 |
| Empty and loading | "No Messages", "No Results" | Skeleton rows only past 150 ms; never a spinner over cached data. Cache construction and size checks are coalesced off the live-message path (at most once per 500 ms, flushing on pagehide/hidden visibility); snapshots stay atomic, owner-scoped and bounded to 2 MiB | — | W2 |
| Archived chats | Not shown | Hidden (`is_archived`) in live-message filters; Recently Deleted follows the recoverable server-filter result | R | W2 |

## 3. Transcript

### 3.1 Bubble anatomy

| Feature | Apple behavior | Web | Bridge | When |
|---|---|---|---|---|
| Colors | Sent iMessage blue (System Blue, white text). Sent SMS/RCS green (System Green). Received light gray (#E9E9EB) or dark (#262628) | Same tokens; the bubble gradient follows the sent stack like iOS (darker toward the top) | R (service per chat) | W2 |
| Shape | Corner radius about 18 pt, a tail on the last bubble of a run, runs grouped within 1 minute from the same sender | Same; tails drawn as one SVG path so they scale cleanly | — | W2 |
| Width | Max about 70% of the transcript width (phone) or 480 px (desktop) | Same | — | W2 |
| Text | Body 17 pt (iOS) / 13 pt (mac, ⌘+/⌘− zoom); data detectors underline links, phones, dates and addresses | Body token; links tappable; phone numbers and addresses linked (`tel:`, maps); ⌘+/⌘− zoom | R (text, parts) | W2 |
| Emoji-only | 1–3 emoji render large with no bubble | Same | R | W2 |
| Sender names in groups | Small name above the first bubble of a run; avatar beside the last | Same | R | W2 |
| Timestamps | Centered date and time headers between gaps over 1 hour; swipe left to reveal exact times on iOS | Same, including the swipe-to-reveal gutter with spring return; desktop hover tooltip | R | W2 |
| Delivered / Read | Under the last sent message: "Delivered", "Read 9:41", "Delivered Quietly" | Same | R (delivered/read); quietly `—` | W2 / W4 |
| Not Delivered | Red "!" with "Not Delivered"; tap to Try Again | Same; Try Again resends | R (`error_code`); resend W | W3 |
| Not sure if delivered | — (Apple has no equivalent; it never loses its own status) | Shown when a send's outcome is indeterminate. Never offers Try Again; offers Check (read-only reconcile by the original command ID) and Copy. If no unmatched outgoing row from this chat appeared while it was pending, the bubble stays with this label. If one did, the bubble is not restored (it may be that row); a transcript pill above the composer reads "1 message may not have been delivered · Check" until reconciliation resolves it. Resolving to observed attaches the command to its row; resolving to a no-effect failure restores the bubble as Not Delivered; a concrete delivery failure uses its captured row. Observed or delivered is always Sent, even without record IDs | indeterminate disposition; `system.command` read | W3 |
| Edited | "Edited" label; tap shows previous versions inline | Same expand animation | R (`edit_history`) | W2 |
| Unsent | Gray system line: "You unsent a message" / "X unsent a message" | Same | R (`retracted_parts`) | W2 |
| Subject line | Bold line above the body (rare) | Same | `—` | W2 |
| Multi-part messages | Text plus attachments as separate stacked bubbles sharing reactions per part | Same | R (`parts`, `part_index`) | W2 |

A per-part `delivery_failed` with a captured record uses that failed row, without restoring a duplicate echo; `failed_no_effect` restores only the failed part’s echo. A `delivery_failed` without a resolvable row keeps its ID absent and resend disabled, and is never reclassified or given a made-up ID. If no unmatched outgoing row was released, its echo stays with "Not Delivered", Copy and no Try Again; if a candidate row was released, the transcript notice reads "1 message wasn't delivered · Check" instead of a second failure bubble.

### 3.2 Item types (common → rare)

| Item | Frequency | Web rendering | Bridge read | When |
|---|---|---|---|---|
| Text | common | Bubble | R | W2 |
| Threaded reply | common | Reply above the original with a connecting curve; "N Replies" opens a focused thread overlay (the background blurs and the thread lifts out) | R (`thread_originator_id`) | W2 |
| Photo (HEIC/JPEG/PNG/WebP/AVIF/GIF) | common | Rounded media bubble at the known aspect ratio. Several photos in a row form a stack (iOS 17+) you can swipe; a grid on desktop. Tap opens the viewer (§9) | R (attachments); dimensions `—` | W2 (with the W1 media pipeline) |
| Link preview | common | Card: image, title, domain; video links play inline when possible | `—` (`balloon_bundle_id`, `payload_data` not read) | W2 read |
| Reactions (tapbacks) | common | Badges on the bubble's top corner: ❤️ 👍 👎 😂 ‼️ ❓, any emoji, or a sticker. Several stack; tap shows who reacted | R (kind, emoji, sender, part) | W2 |
| Video (MOV/MP4/3GP) | occasional | Poster plus duration; plays inline muted on tap-hold, full viewer on tap | R | W2 |
| Mentions | occasional | Bold, tappable name; "@" unseen badge in the list | R (`mention` parts); unseen `—` | W2 |
| Group events | occasional | Gray centered lines: named the conversation, added or removed someone, left, changed the photo | R (`group_event`) | W2 |
| Screen and bubble effects | occasional | **We render them ourselves**, polished beyond Apple's. Bubble: Slam, Loud, Gentle, Invisible Ink (particle shimmer until hovered or tapped). Screen: Echo, Spotlight, Balloons, Confetti, Love, Lasers, Fireworks, Celebration. Plays once on arrival, replay from the menu, off under Reduce Motion | R (`effect`) | W2 |
| Stickers (placed on bubbles) | occasional | Sticker image at its saved position, scale and rotation over the bubble | `—` (`is_sticker`, `sticker_user_info`) | W2 read |
| Memoji, Live and emoji stickers | occasional | Image (animated where the file is) | partial (attachment) | W2 |
| Genmoji | occasional | Inline image glyph sized to the text line | `—` (`emoji_image_*`) | W2 read |
| PDF and other files | occasional | File chip with icon, name and size; opens or downloads | R | W2 |
| Contact card (vCard) | occasional | Card with name and photo; "View" shows the fields | R (attachment) | W2 |
| Audio message | rare | Waveform bubble with play/pause, scrub, duration and speed; transcript text when present; "Keep" on expiring audio | partial (attachment); flags `—` | W2 read, W4 Keep |
| Location / Find My | rare | Captured map link only; opens Maps without a map-tiles service. Live location becomes an honest placeholder | `—` | W2 / placeholder |
| Scheduled (Send Later) | rare | Dashed-outline bubble with "Send Later at 9:00"; edit time, send now, delete | `—` (`schedule_*`) | W4 |
| Live Photo | rare | Still with a LIVE badge; press or hover plays the clip | `—` (pairing) | W2 |
| Text formatting and animated text | rare | Bold, italic, underline and strikethrough render natively. Big, Small, Shake, Nod, Explode, Ripple, Bloom and Jitter are our own animations | `—` (attributedBody attributes not decoded) | W2 read |
| Polls (iOS 26) | rare | Native poll card when the payload decodes; otherwise a placeholder | `—` | W4 / placeholder |
| iMessage app content (games, YouTube, Apple Cash, Photos app) | rare | Placeholder card: app name, caption and image if the payload carries them, "Open on an Apple device" | `—` | placeholder |
| Digital Touch, handwriting | rare | Placeholder card ("Digital Touch: open on an Apple device"); handwriting renders if stored as an image | `—` | placeholder |
| Business chat / RCS cards | rare | Placeholder card with the text content | `—` | placeholder |
| Satellite / SOS / Check In | rare | Label "via satellite"; Check In as a placeholder card | `—` | placeholder |
| Chat background (iOS 26) | — | Show the background image behind the transcript when readable | `—` | W4 / hide |

Videos and Live Photos reserve at least 120 px on the short side, within the available width and height caps, with centered cover cropping. Images and stickers keep the no-upscale rule. Video and Live Photo poster variants target a 360 px short side with a 1.5 MP total budget, reducing the short side only for extreme aspect ratios. Video posters use the original video frame; Live Photo posters use the oriented key still. ThumbHash placeholders are unchanged.

### 3.3 Transcript behavior

- **Opening a chat:** it opens at the first unread message, or at the bottom. A floating "↓ N new" pill appears when new messages arrive above the fold. Catch-up arrow as in iOS 17.
- **Loading history:** scrolling up loads older history in pages with no jump; the scroll anchor stays on the visible message.
- **Selection and copy:** text selection inside a bubble works on desktop; double-click selects a word.
- **Drop to send:** dropping files onto the transcript stages them in the composer.
- **Read receipts:** opening a chat marks it read (mark-read command), respecting the chat's read-receipt setting.

## 4. Composer

| Feature | Apple behavior | Web | Bridge | When |
|---|---|---|---|---|
| Text field | Capsule glass field, grows to 6 lines then scrolls; placeholder "iMessage" or "Text Message" by service | Same; Enter sends, Shift+Enter adds a newline (mac default) | — | W3 |
| Send button | Up-arrow circle in service color, appears when there's content | Same, springs in | send W (proven) | W3 |
| Attachments | "+" menu: Camera, Photos, Stickers, Cash, Audio, Check In, Location, Send Later, Polls, apps | "+" menu: Photos & files (picker, paste, drag), Camera (phone capture), Audio (record → send as file now, as an audio message in W4), Location (browser geolocation → maps link card), Send Later (W4). The rest are hidden | send files W (proven) | W3 / W4 |
| Emoji | System picker | Phone: the OS keyboard. Desktop: our own lightweight picker (⌃⌘Space), searchable, with recents | — | W3 |
| Mentions | Typing a name in a group suggests a member and inserts a mention token | Same suggestion UI | `—` (sending mention attributes) | W4 |
| Reply mode | Swipe right on a bubble or choose Reply; the composer shows the quoted bubble | Same | reply W (proven) | W3 |
| Edit mode | Edit within 15 minutes, up to 5 times; the field turns into an inline edit with ✓ and ✕ | Same limits enforced in the UI | edit W (proven) | W3 |
| Send with effect | Long-press send opens the effect picker | Same picker, our animations preview live | `—` | W4 |
| Formatting | Select text and use B / I / U / S and text effects | Same toolbar | `—` | W4 |
| Optimistic send | Bubble appears instantly, faded until delivered | Same: pending → sent → delivered/read; failure → red "!" | command lifecycle | W3 |
| Drafts | Per-chat draft kept | Same, stored locally per device | — | W3 |

## 5. Message actions

Opened by long-press (touch), right-click (desktop) or the tapback hotkey (⌘T).

| Action | Apple behavior | Web | Bridge | When |
|---|---|---|---|---|
| Tapback picker | The bubble lifts, the background dims and blurs, and a picker of 6 classic reactions + emoji + sticker sits above the bubble with the menu below | Same choreography (§14 motion) | react W (proven, classic + emoji) | W3 |
| Reply | Thread reply | Same | W (proven) | W3 |
| Copy | Copies the text or image | Same (Clipboard API) | — | W2 |
| Edit | Own messages, 15 minutes | Same | W (proven) | W3 |
| Undo Send | Own messages, 2 minutes | Same; the bubble dissolves into particles | W (proven) | W3 |
| Delete | Delete this message for me | Same, with confirm | W sim-only | W3 |
| Forward / More | Select several messages, then forward | Same; forward pre-fills a new message | send W | W3 |
| Save / Download | Save attachment | Download original | blob R | W2 |
| Show times | Exact time | Hover tooltip / swipe gutter | R | W2 |
| Translate | Live Translation (iOS 26) | Hidden (browser translate covers it) | — | hide |

## 6. Conversation details

| Feature | Web | Bridge | When |
|---|---|---|---|
| Header | Avatar (large), name; buttons: Call, FaceTime, Mail open `tel:` / `facetime:` / `mailto:` on the viewer's device; Info opens the contact | R | W2 |
| Group name and photo | Shown; edit in W4 | W4 limited on 25G241; captured photo remains readable | W2 / W4 |
| Members | List with add (+) and remove (swipe) | W4 limited on 25G241; controls hidden without proof | W2 / W4 |
| Hide Alerts | Toggle | `—` | W4 |
| Send Read Receipts | Per-chat toggle | `—` | W4 |
| Share Focus Status | Hidden | — | hide |
| Share My Location | Hidden | — | hide |
| Leave this Conversation | Group only, with confirm | W4 limited on 25G241; control hidden without proof | W4 |
| Delete Conversation | With confirm | W sim-only | W3 |
| Collections | Tabs: Photos (grid), Links, Documents, Locations; from attachments and link balloons | R / `—` (links) | W2 |
| Block / Report Junk | Hidden | — | hide |

## 7. New message

- **To field:** contact autocomplete over names, phones and emails (`contacts.lookup`); tokens; several recipients create a group.
- **Service hint:** shown from history. Blue if this handle has iMessage history, green for SMS/RCS, gray "unknown" otherwise. The bridge can't query Apple's iMessage registry.
- **Existing chat:** if it already exists, the transcript loads under the To field like Apple's.
- **Bridge:** send with handles W (proven to own handles). **When:** W3; group creation in W4.

## 8. Search

- **Field:** global, in the sidebar (desktop) or the bottom glass bar (phone).
- **Filter tokens:** From: person, Photos, Links, Documents, Locations, Unread.
- **Results:** sections for Conversations, Contacts, Messages (with the match highlighted), Photos (grid), Links. Picking a message opens its chat scrolled to it and highlighted with a pulse.
- **Speed:** results while typing within 50 ms of the server responding; recent searches are stored locally.
- **Bridge:** `messages.search` R (literal prefix full-text over messages and attachment filenames). Sender, Photos, Links, Documents, Locations and Unread filters apply server-side before pagination; contact-based sender filters require Contacts visibility. Conversation and contact sections reuse their existing scoped sources. Results are paged in groups of at most 24. Recent searches use the owner-scoped trusted cache; public sessions keep them in memory.
- **When:** W2.

## 9. Media viewer

- **Open:** the image zooms out of its bubble into a full-screen black (dark) or white (light) viewer. Pinch, double-tap and wheel zoom; swipe between all media in the chat; swipe down to dismiss back into the bubble.
- **Video:** custom controls styled like the system player, scrubbing with thumbnails when available, and picture-in-picture.
- **Toolbar:** Share (Web Share API), Download original, Reply, React, Show in Chat.
- **Live Photo:** plays on press.
- **When:** W2.

## 10. Notifications, unread, presence

- **Unread:** tab title "(3) Messages"; the app badge counts unread chats (Badging API); favicon dot.
- **In-app:** a banner slides down for other chats; tapping it opens the chat.
- **Web Push:** opt-in, content-free by default (§DECISIONS).
- **Sounds:** our own subtle send and receive sounds, off by default on desktop.

## 11. Settings (Messages section)

- **Display:** text size (⌘+/⌘−).
- **Sending:** send on Enter.
- **Notifications:** sounds; previews in notifications.
- **Filters:** show Unknown Senders filter.
- Read receipts default: `—`, W4.

## 12. Keyboard shortcuts (desktop, mac-matching)

Keys marked * get checked against the Air's Messages menus in the capture pass. If Apple uses a different key, Apple's key wins.

| Keys | Action |
|---|---|
| ⌘N | New message |
| ⌘F / ⌥⌘F | Search |
| ⌃Tab / ⌃⇧Tab, ⌘1…⌘9 | Next / previous chat, pinned chat N |
| ⌘↑ / ⌘↓ | Top / bottom of transcript |
| ⌘T* | Tapback on the last message |
| ⌘E* | Edit the last sent message |
| ⌘Z after sending | Undo Send (within 2 min) |
| ⌘R* | Reply to the last message |
| ⌘I* | Details |
| ⌘⌫* | Delete conversation (confirm) |
| ⌘+ / ⌘− / ⌘0 | Text size |
| Esc | Close sheet, picker or viewer; cancel edit or reply |

On Windows and Linux, Ctrl stands in for ⌘.

## 13. What needs bridge work (feeds W1/W2/W4 tasks)

**Read model and decoding (W1–W2):**
- `messages.chats`: last message, unread count, muted, pinned order.
- `messages.thread`: pages, reactions folded in, edit history, thread counts.
- Collections queries.
- Decode `balloon_bundle_id` and `payload_data`: link previews, app captions and images, polls.
- Message flags: audio, played, expirable, `schedule_*`, subject, delivered quietly, spam/filtered.
- Recoverable tables (Recently Deleted).
- Stickers: `is_sticker` and placement.
- Genmoji.
- Formatting and text-effect attributes.
- Live Photo pairing.
- Attachment dimensions and duration (media pipeline).

**Writes to prove at the real level (W3):** mark read/unread, delete message, delete chat, resend failed.

**New writes, Accessibility or AppleScript (W4):**
- pin/unpin and reorder
- Hide Alerts
- read receipts per chat
- Send Later (create, edit, cancel)
- send with effect
- formatting and text effects
- mentions
- audio message
- Keep audio
- Recently Deleted recover/delete
- group create, rename, photo, add/remove, leave (iMessage groups need a second iMessage identity; RCS groups via the Ben contact)

**Typing indicators (W4 investigation):** chat.db doesn't store them. Reading may be possible from Messages' Accessibility tree for the open chat only; sending isn't feasible without driving the Mac's UI. Default: **hidden** unless that proves reliable.

## 14. Motion

All motion uses springs that can be interrupted, defined by (response, damping), Apple-style. Every gesture tracks the finger 1:1, and on release it carries its velocity into the spring.

| Moment | Spec |
|---|---|
| Push / pop (phone) | Transcript slides in from the right (response 0.35, damping 1.0) with the list parallaxing at 30% and dimming. The edge swipe is interactive and cancellable |
| Open chat (desktop) | Crossfade plus 8 px rise of the transcript content, 180 ms, no layout shift |
| **Send** (signature) | The bubble grows out of the composer field: shape-morph from the field's capsule into a bubble while rising to its slot (response 0.42, damping 0.82). The previous tail retracts. Other bubbles shift up with a 30 ms stagger |
| Receive | New bubble scales from 0.92 at its tail anchor with a slight overshoot (response 0.38, damping 0.75) |
| **Reaction** (signature) | Long-press: the bubble lifts (scale 1.03, shadow), the backdrop blurs. The picker unfurls from the bubble corner with a stagger of 15 ms per item. Choosing one: the badge flies to the bubble corner along an arc and pops with a small particle burst in the reaction's color |
| Edit / unsend | Edit: the text crossfades in place with a width spring. Unsend: the bubble dissolves into particles drifting up, and the gap closes with a spring |
| **Open media** (signature) | Shared-element zoom from the bubble's rect to fullscreen. The bubble's corner radius springs to 0 and the background fades in. Swipe-down dismiss follows the finger and scales down, then springs back into the bubble |
| Thread overlay | The transcript blurs, and the thread's messages lift into a card with a 20 ms stagger |
| Pinned reorder | Picked-up pin scales 1.1 with a shadow; the others make room with springs |
| Swipe actions | Rubber band past the threshold, snap with a scale pulse on the action icon |
| Effects | Played on a canvas layer above the transcript, GPU-only, ≤ 2.5 s, skippable by tap |
| Reduce Motion | Springs become 150 ms crossfades; effects show a static badge "sent with Confetti" |

## 15. Wording table (preview and system lines)

Ported from Apple's phrasing; localizable keys live in the string catalog.
- **Reactions:** "Loved “…”", "Liked", "Disliked", "Laughed at", "Emphasized", "Questioned", "Reacted 🎉 to …".
- **Reaction removals:** "Removed a heart from “…”", "Removed a like from “…”" and so on; never a preview bubble.
- **Attachments:** "Attachment: N Images", "Video", "Audio Message", "Location", "Contact Card", "Sticker".
- **Group events:** "X named the conversation “…”", "X added Y", "X left the conversation", "X changed the group photo", "X removed the group photo".
- **Unsent:** "You unsent a message".
- **Status:** "Not Delivered", "Delivered Quietly", "Notify Anyway".

## 16. Design tokens (from Apple's 27 kits)

**Colors**

| Token | Light | Dark |
|---|---|---|
| Blue (sent iMessage) | #0088FF | #0091FF |
| Green (sent SMS/RCS) | #34C759 | #30D158 |
| Red | #FF383C | #FF4245 |
| Label primary / secondary / tertiary | #000 / #3C3C43 60% / #3C3C43 30% | #FFF / #EBEBF5 60% / #EBEBF5 30% |
| Fill primary…quaternary | #787878 20% · #787880 16% · #767680 12% · #747480 8% | #787880 36% · 32% · #767680 24% · 18% |
| Background primary / secondary | #FFFFFF / #F2F2F7 | #000000 / #1C1C1E (elevated #1C1C1E / #2C2C2E) |
| Separator | #000 12% (opaque #C6C6C8) | #FFF 12% (opaque #38383A) |
| Grays 1–6 | #8E8E93 #AEAEB2 #C7C7CC #D1D1D6 #E5E5EA #F2F2F7 | #8E8E93 #636366 #48484A #3A3A3C #2C2C2E #1C1C1E |
| mac window background | #FFFFFF | #1E1E1E |

**Type**

| Style | iOS size/line | mac size/line |
|---|---|---|
| Large Title | 34/41 (bold for nav) | 26/32 |
| Title 1 / 2 / 3 | 28/34, 22/28, 20/25 | 22/26, 17/22, 15/20 |
| Headline | 17/22 semibold | 13/16 bold |
| Body | 17/22 | 13/16 |
| Callout | 16/21 | 12/15 |
| Subheadline | 15/20 | 11/14 |
| Footnote | 13/18 | 10/13 |
| Caption 1 / 2 | 12/16, 11/13 | 10/13, 10/13 |

**Fonts:** `system-ui` (SF on Apple devices), Inter elsewhere with tracking tuned to match.

**Materials:** standard materials (mac) are #ECECEC at 38/50/63/76/88% for Ultra Thin through Ultra Thick (light). Liquid Glass Regular is a 70% white fill over a blurred backdrop, with a 0.3 px rim highlight, inner edge shading and an 18/46 px drop shadow at 25% (dark: #1A1A1A 50–90%). The web implementation uses `backdrop-filter: blur(24px) saturate(180%)` plus the rim and shadow layers, only on chrome surfaces.

## 17. Measurements from the real app (pending captures)

Unnamed conversation titles use participants’ given names, then “Alex, Sam & 3 more” for larger groups: verify in capture pass.

To be filled from the Air before W2 starts, for both light and dark:
- bubble radius, padding, tail geometry, group spacing
- avatar sizes, row heights, pinned grid metrics
- sidebar min/max, toolbar height, composer height
- tapback picker geometry and spacing, timestamp header spacing
- link card metrics, media bubble max sizes
- context menu metrics

| To verify | Evidence required | Current rendering |
|---|---|---|
| Native sticker placement coordinates, scale, base size and rotation | Aggregate-only signed Helper distributions, then compare a few real placements in Messages.app during the unlock capture pass | `placement: approximate`: 64 pt at the bubble’s top-trailing corner, unrotated; saved values are preserved without an inferred mapping |

Captures stay in `lab/evidence/private/web/` (gitignored); only the numbers land here.

## 18. Hidden on purpose

- FaceTime and calls (link out only)
- SharePlay
- Apple Cash sending
- Check In sending
- Live Translation
- Share Focus Status and Share My Location
- Block and Report Junk
- iMessage app drawer and games
- Digital Touch and handwriting creation
- Image Playground
- Chat backgrounds editing

## 19. W4 groups, remaining surfaces and typing (2026-10-22)

W4-GROUPS management is **limited** on macOS 26.7.1 (25G241). The installed
`Messages.sdef` has only `send`, `login` and `logout` commands; its chat properties
are `id`, `name` and `account`. It has no group-mutation or typing route.
A group made only of Johan's own handles cannot serve as a proof of an eligible
conversation with distinct participants. Proving with others would text other
people, which the hard rules forbid. The existing real group scenario includes
another contact and must not run under the own-handles-only restriction.

Rename, group photo, add/remove member and leave therefore have no per-build
Accessibility proof. Native management stays fail-closed. The details inspector
keeps captured name, members and group photo visible. Rename, add/remove and
leave controls require fresh operation proofs, an active eligible group and a
live connection. A simulator proof never advertises a native capability. Photo
editing has no proven public route and no control is rendered.

The management forms use trusted input, expected record versions and explicit
confirmation for removing a member or leaving. They never change the displayed
name or membership optimistically. A form closes after the command identifies
the observed chat and its live capture passes the command's commit cursor.
An uncertain outcome stays locked with a read-only Check action; it is never
redispatched automatically. Invalid addresses and duplicate members cannot be
submitted, and revoked proofs prevent further submission.

W4-TYPING is **limited and hidden**. A read-only schema inspection on this build
found no typing field. The scripting dictionary has no typing command or event.
The public Messages framework documents extension conversations and content
insertion, not a macOS typing subscription. Reliable open-chat Accessibility
observation has not been proven with an allowed independent participant; own
aliases cannot establish an incoming remote typing signal. Sending by driving
the native composer per keystroke is not a proven public route and is hidden.
No local composer activity is presented as another person's typing.

W4-UI's remaining surfaces stay truthful: polls retain the existing placeholder,
chat backgrounds stay hidden, and sender-side link-preview generation is
unproven for AppleScript sends and has no composer toggle. Captured incoming
link cards continue to render. Pins/mute/read receipts/Recently Deleted belong
to HELPER-A; Send Later/effects/formatting/mentions/audio/Keep and composer extras
belong to HELPER-B.

References: [Apple's group management guide](https://support.apple.com/guide/messages/icht3268ad23/mac)
and [public Messages framework](https://developer.apple.com/documentation/messages).
