# A compact MCP surface over the full operation catalog

`/mcp` lists 13 tools instead of one tool per operation. Six are general: `apple_status` (access, source readiness, the Mac's time zone and time), `apple_describe` (find operations and fetch one operation's exact contract), `apple_search`, `apple_read`, `apple_change` and `apple_command_status`, plus `apple_guide`. Six cover the most common tasks with flat inputs and name resolution: `messages_read`, `messages_send`, `reminders_add`, `calendar_events`, `contacts_find` and `notes_create`. Listing them and the instructions costs about 3,500 tokens, measured from the recorded `tools/list`; the old list cost about 33,000. The budget test fails above 5,000.

Every catalog operation stays reachable through `apple_read` or `apple_change`, and every old tool name stays callable with its old input and output, unlisted. `messages_send` and `notes_create` collide with old names; a call that carries `args` takes the old path. There is no second endpoint and no code execution: `apple_describe` serves each operation's flattened schema, rules, example and access on request.

The facade calls the same `Service::invoke` as REST and the old tools, so access is still checked per operation. It hides `origin`, `admission_era`, `op`, `expires_at` and `wait_ms`, renames `command_id` to `request_id`, waits up to 15 seconds and returns `status` (`done`, `pending`, `awaiting_approval`, `failed` or `indeterminate`), `request_id`, `ids`, the changed `record` and `next`. Full history needs `verbose`. A send that the Mac only observed locally is never described as delivered. Errors carry `code`, `field`, `problem`, `expected`, `example` and `hint`, including the owner's exact command when access is missing.

The agent-facing descriptions live in `ab-catalog/src/agent.json` beside the frozen descriptors, so `catalog.json` and OpenAPI do not change. Tests require a summary, return description, valid example and description for every parameter; command examples must pass the Mac's validator and the `expected_version` rule must match it. Tool and operation schemas use only portable keywords. Read-only agents are not shown write tools, and each scope adds only its own tool.

Goldens: `mcp-json.json` and `mcp-sse.json` were re-recorded from the Rust frontdoor; `mcp-legacy-tools.json` freezes the old 160-tool list and `mcp-surface.json` freezes the instructions, tools and per-level and per-scope listings.

## Revision: first use by a new agent

`calendar_add` is the 14th tool. `apple_status` is optional; the instructions start from the shortcuts. Listing the tools and instructions costs about 4,200 tokens (o200k tokenizer on the recorded surface); the budget test fails above 6,000 at 3.5 bytes per token.

`apple_describe` ranks by words, not substrings: stopwords are dropped, words are stemmed, a match in the operation's name counts more than one in its summary, and a synonym table maps everyday words (text, SMS, email, screen time, wifi, voicemail, file) to operations and to `records.query` collections that have no operation of their own, such as voicemail, Safari, phone calls and voice memos. A test requires the right operation in the top three for 40 plain requests.

Three operations exist only in the frontdoor, beside `blobs.read` and `blobs.stage`, so `catalog.json` and OpenAPI do not change: `usage.summary` totals Screen Time per device and app from `usage.app_day`, and `calendar.calendars` lists calendars. `calendar.create_event` takes a calendar name; reminder changes take a title instead of `id` and `expected_version`. Names, calendars, lists and folders are matched ignoring case, accents and word order, and several matches return candidates.

Listed tools check their input against their schema: unknown fields, wrong types, out-of-range numbers and unknown search sources are refused before anything runs. Text, note bodies and titles are passed exactly as given; only blank values are refused. Times without an offset are read in the Mac's time zone, dates are accepted where the Mac needs timestamps, and results add `start_local`, `end_local`, `due_local` and `sent_at_local`. A read outside the agent's scopes returns `unauthorized` naming the missing scope instead of an empty result.

A change that partly happened, such as a message whose text was sent and one attachment failed, returns `partial` with each part's outcome and ID and tells the agent not to repeat it with any request ID. `failed` says nothing changed only for proven no-effect outcomes. Both are MCP errors (`isError`) and the audit route records the class.

`apple_search` searches each requested source separately, reports `more` with a continuation per source and `complete`, and never reports no match when the search was cut short or a requested source was not searched.

A Photos change keeps its result in memory for ten minutes, never in the command ledger, which still holds only a request digest. Repeating the call with the same request ID returns that result without running again; after a restart it returns `done` without the result. `apple_command_status` reports Photos changes. The goldens `mcp-json.json`, `mcp-sse.json` and `mcp-surface.json` were re-recorded; `mcp-legacy-tools.json` is unchanged.

## One call for simple reads

Agents spent turns on lookups, not on the bridge: asked how many reminder lists exist, Claude Code described the domain, then the operation, because `time_zone` was listed as required, then read a 3,854-character result. The instructions and `apple_read` now name the common reads with their arguments, `time_zone` is optional wherever the frontdoor fills it in, `reminders.items` takes a list name, and `reminders.lists` returns only titles, counts, IDs and versions (`verbose` returns the full result). `screen_time` and `photos_find` are the 15th and 16th tools; `photos_find` runs `photos.search` and `photos.contact_sheet` and returns the image with the numbered asset IDs. On nine simple questions in a synthetic world, Claude Code (Opus 5.5) went from 41 model turns to 29, with every question but one at the three-turn minimum (tool search, one call, answer). Listing the tools and instructions costs about 4,900 tokens (o200k).

Messages from `messages_read` and `apple_search` carry `from` (the contact's name, the address without a contact, or `me`) and `chat_title`, resolved through `messages.handles` and the chat record. `apple_describe` treats a query about looking, finding, showing or reading, with no change or capture words, as a read: changing and capture operations drop and reads in the named domain rise.

An agent's `request_id` is scoped to that agent: the command ID is the readable request ID followed by a hash of the principal and the full request ID, and results show the agent its own ID. A change admitted before this under the raw ID is still found by that agent, so a retry never acts twice. A read-only agent cannot be given write access, so its refusals say to connect a new agent, and `bridge agents update` refuses write scopes for a read agent.


## Showing a photo and finding more without new tools

`photos_show` is the 17th tool: one photo as the photo itself, full resolution, for the person, with `save` for the original file. `photos_find` tells agents its grid is their private index and returns the photo itself for a single match, and `messages_send` takes `photos`. To stay under the 6,000-token budget, sentences that repeat the instructions were removed from `apple_read`, `apple_change`, `messages_read` and `apple_describe`, and the two Photos tools return a plain object schema; listing now measures 6,000 tokens by the test's 3.5-bytes-per-token rule, exactly the budget, so the next tool needs room made first.

`calendar.free_slots` and `messages.shared` are frontdoor-only reads reached through `apple_read` and found by `apple_describe`, so the list does not grow: free time between timed events within working hours, and a chat's attachments by kind and links with their senders.

## Revision: Safari, Shortcuts and Mac settings removed

[ADR 0033](0033-remove-safari-shortcuts-and-mac-settings.md) removed those sources. `mcp-legacy-tools.json` now freezes 138 old tools, `apple_search` no longer names Safari, and the synonym table no longer maps browsing history, now playing or Time Machine to an operation.

## Revision: agent operation names

[ADR 0038](0038-agent-operation-names.md) renamed the operations agents see to `app.verb_object`, listed every name in the `apple_read` and `apple_change` schemas and removed the unlisted old tools and `mcp-legacy-tools.json`. Old names are refused with the new name as a hint. Catalog, REST, CLI and Helper names are unchanged.
