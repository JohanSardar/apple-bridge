# Small, paged answers for agents

Agents got answers far larger than the question. On the owner's Mac, `notes.list` returned 1.2 MB, reminders for today 49 KB and unread chats 32 KB, which was mostly spam and unknown senders. The calendar for today also missed an all-day event. Answers are now sized and paged by the MCP facade. The catalog operations, REST and `catalog.json` do not change.

## Budgets

Each answer is measured at 3.5 bytes per token, the same rule as the connect budget. A default read answer stays under about 2,000 tokens. An answer the agent asked to be larger stays under about 8,000. That covers `verbose`, `detail`, a `limit` above 20, a batch of reads and the documentation tools. Lists default to 20 items and accept at most 100.

Reads of one whole item are never trimmed: `records.get`, `mail.body`, `files.read` and `blobs.read`. Clipped note text could be written back with `notes.update` and lose the end of the note. A clipped blob chunk would corrupt the file, because its `length` and `offset` still describe the full chunk. A blob chunk is as large as the agent asked for, and a single record is as large as the record. Those are the only answers that may pass 8,000 tokens.

When an answer is still over its budget, the facade clips the longest text over 1,500 characters, or drops items from the end of the largest list, until it fits. It then adds `cut` with the reason, `dropped_items` per list and `shortened_text` per field. The page it trimmed gets a corrected `returned`, `truncated: true` and a `next` that resumes at the first dropped item. When the dropped items are in a list with no page, as in a `verbose` answer, `cut.unpaged` says how to see them. A text that cannot be shortened further is left alone, and trimming moves on to lists. Nothing is dropped without saying so.

## Pages

Every list carries `page`: `total`, `returned`, `truncated`, `narrow` (how to ask for less), and `next` when there is more. `next` is either `{offset}` or `{arguments, offset}`. `{arguments, offset}` is used when the read model returned a cursor, which is taken from the operation's raw result, never from the facade's hint text. When the read model has more than it returned, `total` reads `more than N`. Two lists in one answer use `pages`, keyed by list name, or `<list>_page` beside the list. A chat list scans up to 1,000 chats. When there are more, its last page is `truncated`, and `next` gives the raw `messages.chats` call with the cursor. A search that has more in some source gives that source's continuation in `next`, and every source's continuation in `more`. `apple_read`, `messages_read` and `calendar_events` take `offset`. For `apple_read`, the operation's own `limit` in `arguments` sets the page size.

## Defaults

- **Calendar.** A one-day query includes all-day and multi-day events that cover the day. The Helper now records an all-day event's end as the day after its last day, which matches writes, the sim and lab-demo. EventKit gives the last second instead. The agenda read model also accepts the old stored form, an end on the last day itself or equal to the start, so a one-day event in an existing store is listed without a new capture. The old form of a multi-day event cannot be told apart from the new form of an event one day shorter. Until the Helper captures it again, such an event is missing on its last day, as it was before this change. `calendar.range`, which `calendar_events` uses with `text`, follows the same rule. `calendar_events` returns short cards, and `detail: true` adds notes, attendees and alarms.
- **Messages.** Chat lists leave out spam and chats whose senders are not in Contacts. With `unread`, they also leave out unread chats older than 30 days. `omitted` counts each group and gives the call that shows them (`senders: all`). Chat cards keep the title, unread count, last activity, and the last message's sender, text and time. Handle IDs stay in `verbose`.
- **Reminders.** `smart: today` returns `due_today` (paged), `overdue` (count plus the latest three) and `upcoming` (count plus the next five within seven days). Each group has a `show_all` call, and `view` names one group to page in full.
- **Notes.** `notes.list` returns titles, folder names, dates and a 100-character preview, plus the folder list. Bodies, HTML and blocks come only from `records.get`.
- **Contacts and search.** `contacts_find` returns name, organization, phones, emails and birthday. `apple_search` defaults to five items per source and returns cards.

This adds no combined daily call. Sending is unchanged.

## Measuring

The frontdoor audit line for each MCP call carries `answer_bytes`, a size and never content. `bridge stats --answers` ranks tool and operation pairs by their largest answer, with call count and median, and `--json` prints the same.

`ab-runtime/tests/agent_answers.rs` runs a gateway on a large synthetic Mac. The Mac has lab-demo plus 400 heavy notes, 30 events today across 22 test calendars, an all-day event with an end equal to its start, a multi-day all-day event, 60 overdue reminders, 330 stranger chats and 300 mails. The test calls the default reads over `/mcp` and fails when:

- an answer goes over its budget,
- a list goes over 100 items,
- a page lacks its fields or is cut without `truncated`,
- a cursor is a sentence,
- or the all-day or multi-day event is missing today.

Reverting the agenda change makes it fail.

On 2026-11-17 the branch binary served a private, read-only copy of the owner's store with no Helper. Every default answer came in under 2,000 tokens, and today's all-day event was listed. The sizes are in `lab/evidence/agent-answers-real-store.json`.

## Instructions

Item 4 of the MCP instructions now states the page rule. To stay within the 6,000-token connect budget, `apple_read` lost its top-level `limit` description and `calendar_events` its `limit`, and several tool descriptions were shortened. `mcp-surface.json`, `mcp-json.json` and `mcp-sse.json` were re-recorded.
