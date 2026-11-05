# Operations reference

Generated from the operation registry in `rust/crates/ab-catalog` with `cargo run -p ab-catalog --bin operations-reference`. Do not edit by hand.

This lists the catalog only: whether an operation is usable depends on its source, granted permissions, and the feature status in `features.yaml`.

147 operations.

## audio

### audio.transcribe

On-device transcription without authorization prompts

- Kind: `query`
- Scope: `audio:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `audio.transcribe` |

## calendar

### calendar.agenda

Read local occurrences overlapping a time range; all-day dates remain floating

- Kind: `read`
- Scope: `calendar:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `calendar_id` | string | no |  |
| `end` | string | yes |  |
| `start` | string | yes |  |

### calendar.range

Read a half-open Calendar range with floating dates, calendar filters and precomputed overlap groups

- Kind: `read`
- Scope: `calendar:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `calendars` | array<string> | no |  |
| `end` | string | yes |  |
| `start` | string | yes |  |
| `text` | string | no |  |
| `time_zone` | string | no |  |

### calendar.create_event

Calendar create_event; update and delete require expected_version and span this or future

- Kind: `command`
- Scope: `calendar:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### calendar.update_event

Calendar update_event; update and delete require expected_version and span this or future

- Kind: `command`
- Scope: `calendar:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### calendar.delete_event

Calendar delete_event; update and delete require expected_version and span this or future

- Kind: `command`
- Scope: `calendar:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### calendar.create_calendar

Calendar create_calendar; update and delete require expected_version and span this or future

- Kind: `command`
- Scope: `calendar:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## capture

### capture.play_sound

Play a named system sound with finished callback evidence

- Kind: `command`
- Scope: `capture:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### capture.camera_photo

Capture an artifact delivered through the blob store

- Kind: `command`
- Scope: `capture:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### capture.record_audio

Capture an artifact delivered through the blob store

- Kind: `command`
- Scope: `capture:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### capture.screenshot

Capture an artifact delivered through the blob store

- Kind: `command`
- Scope: `capture:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## contacts

### contacts.list

Read grouped contacts in deterministic A–Z sections with viewer name preferences, identity, avatar references and scope-filtered linked conversations

- Kind: `read`
- Scope: `contacts:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `display_order` | string | no | One of: `given`, `family` |
| `group` | string | no |  |
| `sort` | string | no | One of: `given`, `family` |

### contacts.lookup

Look up unified contacts by partial q (name, phone or email), capped at 25 in deterministic name/ID order; legacy name and exact handle filters remain supported

- Kind: `read`
- Scope: `contacts:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `handle` | string | no |  |
| `limit` | integer | no | New q searches default to 25; positive limits are capped at 25 |
| `name` | string | no |  |
| `q` | string | no | Partial name, normalized phone or email; 1–64 characters after trimming |

### contacts.create

Create a contact in the default writable container

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.update

Patch contact fields with expected_version

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.delete

Delete a writable contact with expected_version

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.create_group

Create a contact group in the default container

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.rename_group

Rename a writable contact group with expected_version

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.delete_group

Delete a writable contact group without deleting its contacts

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.add_member

Add a contact from the same container to a group

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### contacts.remove_member

Remove group membership without deleting the contact

- Kind: `command`
- Scope: `contacts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## files

### files.search

Search opted-in file roots

- Kind: `query`
- Scope: `files:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `files.search` |

### files.read

Read an opted-in file

- Kind: `query`
- Scope: `files:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `files.read` |

### files.create

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.write

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.move

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.rename

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.trash

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.set_tags

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.download

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.evict

Mutate an opted-in file with native readback

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### files.transform

Transform staged image and PDF blobs natively

- Kind: `command`
- Scope: `files:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## focus

### focus.set

Set Do Not Disturb through AB Focus and confirm Focus status; agents require approval

- Kind: `command`
- Scope: `focus:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## gateway

### search

Search visible local record text

- Kind: `read`
- Scope: none

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mac.app`, `mac.audio_device`, `mac.bluetooth_device`, `mac.display`, `mac.network`, `mac.power`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `safari.bookmark`, `safari.cloud_tab`, `safari.history`, `safari.reading_list`, `safari.tab`, `shortcuts.folder`, `shortcuts.shortcut`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
| `id` | string | no |  |
| `include_aliases` | boolean | no |  |
| `include_deleted` | boolean | no |  |
| `limit` | integer | no |  |
| `text` | string | no |  |

### records.get

Read one visible local bridge record

- Kind: `read`
- Scope: none

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mac.app`, `mac.audio_device`, `mac.bluetooth_device`, `mac.display`, `mac.network`, `mac.power`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `safari.bookmark`, `safari.cloud_tab`, `safari.history`, `safari.reading_list`, `safari.tab`, `shortcuts.folder`, `shortcuts.shortcut`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
| `id` | string | no |  |
| `include_aliases` | boolean | no |  |
| `include_deleted` | boolean | no |  |
| `limit` | integer | no |  |
| `text` | string | no |  |

### records.query

Page visible local bridge records in collection and ID order

- Kind: `read`
- Scope: none

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mac.app`, `mac.audio_device`, `mac.bluetooth_device`, `mac.display`, `mac.network`, `mac.power`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `safari.bookmark`, `safari.cloud_tab`, `safari.history`, `safari.reading_list`, `safari.tab`, `shortcuts.folder`, `shortcuts.shortcut`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
| `id` | string | no |  |
| `include_aliases` | boolean | no |  |
| `include_deleted` | boolean | no |  |
| `limit` | integer | no |  |
| `text` | string | no |  |

### sources.status

Read source coverage and freshness within scope

- Kind: `read`
- Scope: none

No parameters.

### commands.status

Look up an issued command by its original ID; resolve only gathers stronger evidence

- Kind: `read`
- Scope: none

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `command_id` | string | no |  |
| `limit` | integer | no |  |
| `resolve` | boolean | no |  |

### commands.list

List commands issued by this principal within source scope

- Kind: `read`
- Scope: none

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `command_id` | string | no |  |
| `limit` | integer | no |  |
| `resolve` | boolean | no |  |

## health

### health.summary

Day or week summary of captured Health samples, grouped by type and unit; durations clipped to the window, quantities assigned by start time

- Kind: `read`
- Scope: `health:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `date` | string | yes |  |
| `device_id` | string | no |  |
| `period` | string | yes |  |
| `time_zone` | string | no |  |

## iphone

### iphone.alarm.cancel

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.alarm.create

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.alarm.list

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.alarm.update

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.capture.audio

Record up to ten seconds after on-phone Face ID approval and upload to the gateway

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.capture.photo

Capture a photo after on-phone Face ID approval and upload to the gateway

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.device.status

Read iPhone device.status through the paired device

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.devices

List paired iPhones; revoke a device with the gateway revoke command

- Kind: `read`
- Scope: `iphone:read`

No parameters.

### iphone.geofence.add

Monitor a named geofence

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.geofence.list

Publish named geofences as iPhone location records

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.geofence.remove

Remove a named geofence

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.health.log

Log tagged water (ml), weight (kg), mindful minutes or workout minutes on the paired iPhone

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.live.end

Push running agent task status to the iPhone lock screen

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.live.start

Push running agent task status to the iPhone lock screen

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.live.update

Push running agent task status to the iPhone lock screen

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.location.current

Read precise location into an iPhone location record

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.location.significant

Enable or disable significant location change records

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.motion.activity

Read authorized iPhone motion activity

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.motion.history

Read authorized iPhone motion history

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.notify

Deliver a notification; with buttons, return the selected action as the signed command result

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.ping

Round-trip a command through the paired iPhone and return its signed result

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.screentime.cancel

Cancel Bridge-owned Screen Time controls on the paired iPhone

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.screentime.schedule

Schedule Bridge-owned Screen Time controls on the paired iPhone

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.screentime.shield

Shield Bridge-owned Screen Time controls on the paired iPhone

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.screentime.unshield

Unshield Bridge-owned Screen Time controls on the paired iPhone

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.timer.cancel

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### iphone.timer.start

Operate app-owned iPhone AlarmKit alarms and timers

- Kind: `command`
- Scope: `iphone:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## location

### location.current

Transient location.current result

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.current` |

### location.geocode

Transient location.geocode result

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.geocode` |

### location.reverse_geocode

Transient location.reverse_geocode result

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.reverse_geocode` |

## mac

### mac.set_volume

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.set_mute

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.set_appearance

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.launch_app

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.activate_app

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.quit_app

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.keep_awake

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.clipboard_write

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.notify

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.open_settings

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.bluetooth_devices

Read Mac extras; native availability is explicit

- Kind: `query`
- Scope: `mac:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mac.bluetooth_devices` |

### mac.clipboard_read

Read transient clipboard text without capturing contents

- Kind: `query`
- Scope: `mac:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mac.clipboard_read` |

### mac.iwork_export

Mac extras action with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.lock_screen

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.network_check

Check default network reachability, DNS and interface counters

- Kind: `query`
- Scope: `mac:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mac.network_check` |

### mac.recognize_song

Read Mac extras; native availability is explicit

- Kind: `query`
- Scope: `mac:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mac.recognize_song` |

### mac.restart

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.screen_record

Record up to 30 seconds of screen video as a blob

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.set_wallpaper

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.shutdown

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.sleep_display

Change Mac state with native readback

- Kind: `command`
- Scope: `mac:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.time_machine_status

Read Mac extras; native availability is explicit

- Kind: `query`
- Scope: `mac:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mac.time_machine_status` |

## mail

### mail.body

Read local Mail body

- Kind: `query`
- Scope: `mail:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `mail.body` |

### mail.create_draft

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mail.send

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mail.move

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mail.set_flag

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mail.mark_read

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mail.delete

Mail effect with readback evidence

- Kind: `command`
- Scope: `mail:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## maps

### maps.open

Open Maps at a coordinate; verify Maps is frontmost

- Kind: `command`
- Scope: `maps:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### maps.snapshot

Map snapshot PNG

- Kind: `query`
- Scope: `maps:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `maps.snapshot` |

### maps.search

Transient maps.search result

- Kind: `query`
- Scope: `maps:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `maps.search` |

### maps.nearby

Transient maps.nearby result

- Kind: `query`
- Scope: `maps:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `maps.nearby` |

### maps.directions

Transient maps.directions result

- Kind: `query`
- Scope: `maps:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `maps.directions` |

### maps.eta

Estimate travel time and distance between from/to points with numeric lat and lon fields. Returns duration in milliseconds and distance in meters.

- Kind: `query`
- Scope: `maps:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `maps.eta` |

## messages

### messages.chats

Page visible local chat summaries by newest activity, optionally matching an exact normalized participant handle set through indexes; sender filters, unread and unseen mentions; mute and pinned state remain nullable

- Kind: `read`
- Scope: `messages:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no | Opaque cursor from the previous page |
| `filter` | string | no | all (default), known, unknown, unread, deleted, spam, or archived |
| `limit` | integer | no | Page size, default 50, maximum 200 |
| `participants` | array<string> | no | Exact normalized phone/email handle set, 1–32 inputs; duplicates folded |

### messages.thread

Read a local chat in sent_at and ID order with current reactions, complete attachments and reply counts; opaque before/after cursors, latest, oldest, unread or message anchors, and an optional thread_originator_id filter for a root and all its replies

- Kind: `read`
- Scope: `messages:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `anchor` | string | no | latest (default), oldest, unread, or a message ID |
| `before` | string | no |  |
| `chat_id` | string | yes |  |
| `limit` | integer | no | Page size, default 50, maximum 200 |
| `thread_originator_id` | string | no | Restrict to this root message and its replies in the chat |

### messages.handles

Resolve up to 500 visible handles with contact names, initials and avatars

- Kind: `read`
- Scope: `messages:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `ids` | array<string> | yes | At most 500 handle IDs |

### messages.search

Search visible local messages and attachment filenames with prefix text, sender, media-kind and unread filters; filters apply before pagination

- Kind: `read`
- Scope: `messages:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `from_contact_id` | string | no |  |
| `from_handle_id` | string | no |  |
| `kind` | string | no | One of: ``, `photos`, `links`, `documents`, `locations` |
| `limit` | integer | no |  |
| `text` | string | no |  |
| `unread` | boolean | no |  |

### messages.collection

Page visible chat attachments or balloon messages by media kind

- Kind: `read`
- Scope: `messages:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `after` | string | no |  |
| `chat_id` | string | yes |  |
| `kind` | string | yes | One of: `photos`, `videos`, `media`, `links`, `documents`, `locations`, `audio` |
| `limit` | integer | no |  |

### messages.send

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.reply

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.react

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.edit

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.unsend

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.mark_read

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.mark_unread

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.delete_message

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### messages.delete_chat

Messages command with evidence; unproven native routes reject before dispatch

- Kind: `command`
- Scope: `messages:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## notes

### notes.list

Read Notes accounts, nested folders, pinned and date-grouped notes with previews and thumbnail references; locked notes are title-only and Recently Deleted is read-only

- Kind: `read`
- Scope: `notes:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `folder` | string | no |  |
| `sort` | string | no | One of: `modified`, `created`, `title` |
| `time_zone` | string | no |  |

### notes.create

Create a plain text note in a folder

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### notes.update

Replace or append plain text, or edit text in canonical rich HTML with expected_version; rich HTML structure must be preserved

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### notes.delete

Delete a note with expected_version

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### notes.create_folder

Create a Notes folder

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### notes.move

Move a note to a writable folder with expected_version

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### notes.rename_folder

Rename a tagged Bridge Test folder with expected_version

- Kind: `command`
- Scope: `notes:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## pdf

### pdf.text

Transient pdf.text result

- Kind: `query`
- Scope: `pdf:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `pdf.text` |

### pdf.search

Transient pdf.search result

- Kind: `query`
- Scope: `pdf:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `pdf.search` |

### pdf.annotate

Create an annotated PDF artifact

- Kind: `command`
- Scope: `pdf:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## print

### print.document

Print an authorized blob or supplied PDF bytes to PDF or a named printer

- Kind: `command`
- Scope: `print:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## reminders

### reminders.items

Read visible reminders with sections, subtasks and attachments in the viewer time zone

- Kind: `read`
- Scope: `reminders:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `list` | string | no |  |
| `show_completed` | boolean | no |  |
| `smart` | string | no | One of: `today`, `scheduled`, `all`, `flagged`, `completed`, `assigned` |
| `sort` | string | no | One of: `manual`, `due`, `created`, `priority`, `title` |
| `tag` | string | no |  |
| `time_zone` | string | yes |  |

### reminders.lists

Read native list and group order, tags and smart counts in the viewer time zone; unsupported counts are omitted

- Kind: `read`
- Scope: `reminders:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `time_zone` | string | yes | Viewer IANA time zone |

### reminders.create

Create a reminder in a writable list

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.create_list

Create a writable Reminders list

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.update

Patch reminder fields with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.complete

Complete a reminder with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.uncomplete

Reopen a reminder with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.delete

Delete a reminder with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.rename_list

Rename a Reminders list with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### reminders.delete_list

Delete a Reminders list with expected_version

- Kind: `command`
- Scope: `reminders:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## shortcuts

### shortcuts.run

Run a shortcut with text input; evidence establishes CLI completion only

- Kind: `command`
- Scope: `shortcuts:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### shortcuts.actions

Read bounded installed App Intents metadata; no executable schema guarantee

- Kind: `query`
- Scope: `shortcuts:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `shortcuts.actions` |

## speech

### speech.synthesize

Transient synthesized WAV audio

- Kind: `query`
- Scope: `speech:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `speech.synthesize` |

### speech.say

Speak text with finished callback evidence

- Kind: `command`
- Scope: `speech:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## system

### replica.status

Read local replication and blob progress

- Kind: `read`
- Scope: `system:read`

No parameters.

### helper.version

Read transient Helper state without world effects

- Kind: `query`
- Scope: `system:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `helper.version` |

## vision

### vision.ocr

Transient vision.ocr result

- Kind: `query`
- Scope: `vision:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `vision.ocr` |

