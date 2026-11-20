# Operations reference

Generated from the operation registry in `rust/crates/ab-catalog` with `cargo run -p ab-catalog --bin operations-reference`. Do not edit by hand.

This lists the catalog only: whether an operation is usable depends on its source, granted permissions, and the feature status in `features.yaml`.

`bridge permissions status --json` reads source readiness without prompting. Values are `granted`, `needs_permission` and `unknown`; `needs_permission` does not establish that a request was denied.

138 operations.

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
| `args.alarms` | array<object> \| null | no |  |
| `args.alarms[].absolute_date` | string | no |  |
| `args.alarms[].relative_offset` | number \| null | no |  |
| `args.all_day` | boolean \| null | no |  |
| `args.availability` | integer \| null | no |  |
| `args.calendar_id` | string | yes |  |
| `args.end` | string \| null | yes |  |
| `args.location` | string \| null | no |  |
| `args.notes` | string \| null | no |  |
| `args.recurrence` | array<object> \| null | no |  |
| `args.recurrence[].count` | integer | no |  |
| `args.recurrence[].end` | string | no |  |
| `args.recurrence[].frequency` | string | yes |  |
| `args.recurrence[].interval` | integer | yes |  |
| `args.start` | string \| null | yes |  |
| `args.time_zone` | string \| null | no |  |
| `args.title` | string \| null | yes |  |
| `args.url` | string \| null | no |  |
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
| `args.alarms` | array<object> \| null | no |  |
| `args.alarms[].absolute_date` | string | no |  |
| `args.alarms[].relative_offset` | number \| null | no |  |
| `args.all_day` | boolean \| null | no |  |
| `args.availability` | integer \| null | no |  |
| `args.calendar_id` | string | no |  |
| `args.end` | string \| null | no |  |
| `args.id` | string | yes |  |
| `args.location` | string \| null | no |  |
| `args.notes` | string \| null | no |  |
| `args.recurrence` | array<object> \| null | no |  |
| `args.recurrence[].count` | integer | no |  |
| `args.recurrence[].end` | string | no |  |
| `args.recurrence[].frequency` | string | yes |  |
| `args.recurrence[].interval` | integer | yes |  |
| `args.span` | string | yes | One of: `this`, `future` |
| `args.start` | string \| null | no |  |
| `args.time_zone` | string \| null | no |  |
| `args.title` | string \| null | no |  |
| `args.url` | string \| null | no |  |
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
| `args.id` | string | yes |  |
| `args.span` | string | yes | One of: `this`, `future` |
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
| `args.title` | string \| null | yes |  |
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
| `args.name` | string | yes | One of: `Basso`, `Blow`, `Bottle`, `Frog`, `Funk`, `Glass`, `Hero`, `Morse`, `Ping`, `Pop`, `Purr`, `Sosumi`, `Submarine`, `Tink` |
| `args.volume` | number | no |  |
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
| `args.seconds` | number | yes |  |
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
| `args.display` | integer \| null | no |  |
| `args.window` | integer \| null | no |  |
| `command_id` | string | no |  |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

### mac.screen_record

Record up to 30 seconds of screen video as a blob

- Kind: `command`
- Scope: `capture:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `args.seconds` | number | yes |  |
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
| `args.birthday` | object \| null | no |  |
| `args.birthday.calendar` | string | no |  |
| `args.birthday.day` | integer | yes |  |
| `args.birthday.is_leap_month` | boolean | no |  |
| `args.birthday.month` | integer | yes |  |
| `args.birthday.year` | integer | no |  |
| `args.dates` | array<object> | no |  |
| `args.dates[].label` | string | no |  |
| `args.dates[].value` | object \| null | yes |  |
| `args.dates[].value.calendar` | string | no |  |
| `args.dates[].value.day` | integer | yes |  |
| `args.dates[].value.is_leap_month` | boolean | no |  |
| `args.dates[].value.month` | integer | yes |  |
| `args.dates[].value.year` | integer | no |  |
| `args.department` | string | no |  |
| `args.emails` | array<object> | no |  |
| `args.emails[].label` | string | no |  |
| `args.emails[].value` | string | yes |  |
| `args.family_name` | string | no |  |
| `args.given_name` | string | no |  |
| `args.image` | object \| null | no |  |
| `args.image.base64` | string | yes | Image bytes in base64, 8 MB at most. |
| `args.instant_messages` | array<object> | no |  |
| `args.instant_messages[].label` | string | no |  |
| `args.instant_messages[].value` | object | yes |  |
| `args.job_title` | string | no |  |
| `args.middle_name` | string | no |  |
| `args.name_prefix` | string | no |  |
| `args.name_suffix` | string | no |  |
| `args.nickname` | string | no |  |
| `args.non_gregorian_birthday` | object \| null | no |  |
| `args.non_gregorian_birthday.calendar` | string | no |  |
| `args.non_gregorian_birthday.day` | integer | yes |  |
| `args.non_gregorian_birthday.is_leap_month` | boolean | no |  |
| `args.non_gregorian_birthday.month` | integer | yes |  |
| `args.non_gregorian_birthday.year` | integer | no |  |
| `args.organization` | string | no |  |
| `args.phones` | array<object> | no |  |
| `args.phones[].label` | string | no |  |
| `args.phones[].value` | string | yes |  |
| `args.phonetic_family_name` | string | no |  |
| `args.phonetic_given_name` | string | no |  |
| `args.phonetic_middle_name` | string | no |  |
| `args.phonetic_organization` | string | no |  |
| `args.postal_addresses` | array<object> | no |  |
| `args.postal_addresses[].label` | string | no |  |
| `args.postal_addresses[].value` | object | yes |  |
| `args.previous_family_name` | string | no |  |
| `args.relations` | array<object> | no |  |
| `args.relations[].label` | string | no |  |
| `args.relations[].value` | string | yes |  |
| `args.social_profiles` | array<object> | no |  |
| `args.social_profiles[].label` | string | no |  |
| `args.social_profiles[].value` | object | yes |  |
| `args.urls` | array<object> | no |  |
| `args.urls[].label` | string | no |  |
| `args.urls[].value` | string | yes |  |
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
| `args.birthday` | object \| null | no |  |
| `args.birthday.calendar` | string | no |  |
| `args.birthday.day` | integer | yes |  |
| `args.birthday.is_leap_month` | boolean | no |  |
| `args.birthday.month` | integer | yes |  |
| `args.birthday.year` | integer | no |  |
| `args.dates` | array<object> | no |  |
| `args.dates[].label` | string | no |  |
| `args.dates[].value` | object \| null | yes |  |
| `args.dates[].value.calendar` | string | no |  |
| `args.dates[].value.day` | integer | yes |  |
| `args.dates[].value.is_leap_month` | boolean | no |  |
| `args.dates[].value.month` | integer | yes |  |
| `args.dates[].value.year` | integer | no |  |
| `args.department` | string | no |  |
| `args.emails` | array<object> | no |  |
| `args.emails[].label` | string | no |  |
| `args.emails[].value` | string | yes |  |
| `args.family_name` | string | no |  |
| `args.given_name` | string | no |  |
| `args.id` | string | yes |  |
| `args.image` | object \| null | no |  |
| `args.image.base64` | string | yes | Image bytes in base64, 8 MB at most. |
| `args.instant_messages` | array<object> | no |  |
| `args.instant_messages[].label` | string | no |  |
| `args.instant_messages[].value` | object | yes |  |
| `args.job_title` | string | no |  |
| `args.middle_name` | string | no |  |
| `args.name_prefix` | string | no |  |
| `args.name_suffix` | string | no |  |
| `args.nickname` | string | no |  |
| `args.non_gregorian_birthday` | object \| null | no |  |
| `args.non_gregorian_birthday.calendar` | string | no |  |
| `args.non_gregorian_birthday.day` | integer | yes |  |
| `args.non_gregorian_birthday.is_leap_month` | boolean | no |  |
| `args.non_gregorian_birthday.month` | integer | yes |  |
| `args.non_gregorian_birthday.year` | integer | no |  |
| `args.organization` | string | no |  |
| `args.phones` | array<object> | no |  |
| `args.phones[].label` | string | no |  |
| `args.phones[].value` | string | yes |  |
| `args.phonetic_family_name` | string | no |  |
| `args.phonetic_given_name` | string | no |  |
| `args.phonetic_middle_name` | string | no |  |
| `args.phonetic_organization` | string | no |  |
| `args.postal_addresses` | array<object> | no |  |
| `args.postal_addresses[].label` | string | no |  |
| `args.postal_addresses[].value` | object | yes |  |
| `args.previous_family_name` | string | no |  |
| `args.relations` | array<object> | no |  |
| `args.relations[].label` | string | no |  |
| `args.relations[].value` | string | yes |  |
| `args.social_profiles` | array<object> | no |  |
| `args.social_profiles[].label` | string | no |  |
| `args.social_profiles[].value` | object | yes |  |
| `args.urls` | array<object> | no |  |
| `args.urls[].label` | string | no |  |
| `args.urls[].value` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.name` | string | yes |  |
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
| `args.id` | string | yes |  |
| `args.name` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.contact_id` | string | yes |  |
| `args.id` | string | yes | The group. |
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
| `args.contact_id` | string | yes |  |
| `args.id` | string | yes | The group. |
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
| `args.content_base64` | string | yes |  |
| `args.digest` | string | yes |  |
| `args.name` | string | yes |  |
| `args.parent` | string | no |  |
| `args.root` | string | yes |  |
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
| `args.content_base64` | string | yes |  |
| `args.digest` | string | yes |  |
| `args.name` | string | yes |  |
| `args.parent` | string | no |  |
| `args.root` | string | yes |  |
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
| `args.name` | string | yes |  |
| `args.parent` | string | no |  |
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
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
| `args.name` | string | yes |  |
| `args.parent` | string | no |  |
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
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
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
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
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
| `args.tags` | array<string> | yes |  |
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
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
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
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
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
| `args.path` | string | yes |  |
| `args.root` | string | yes |  |
| `command_id` | string | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## focus

### focus.set

Set Do Not Disturb through AB Focus and confirm Focus status

- Kind: `command`
- Scope: `focus:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `args.mode` | string | yes | One of: `Do Not Disturb` |
| `args.on` | boolean | yes |  |
| `args.until` | string | no |  |
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
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
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
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
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
| `collection` | string | yes | Collection name; use an empty string to query all visible collections. Follow next with after to read further pages. One of: ``, `calendar.calendar`, `calendar.event`, `contacts.contact`, `contacts.container`, `contacts.group`, `files.item`, `focus.status`, `health.sample`, `iphone.location`, `mail.account`, `mail.attachment`, `mail.mailbox`, `mail.message`, `messages.attachment`, `messages.chat`, `messages.handle`, `messages.message`, `messages.reaction`, `notes.folder`, `notes.note`, `pdf.artifact`, `phone.call`, `reminders.attachment`, `reminders.list`, `reminders.reminder`, `system.command`, `system.replica`, `system.source`, `usage.app_day`, `usage.device`, `voice_memos.folder`, `voice_memos.recording`, `voicemail.message` |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.id` | string | yes | UUID of the alarm or timer. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.fire_at` | number | yes | Unix seconds, in the future. |
| `args.id` | string | no | UUID of the alarm or timer. |
| `args.snooze_seconds` | number \| null | no |  |
| `args.title` | string | no |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.fire_at` | number | yes | Unix seconds, in the future. |
| `args.id` | string | yes | UUID of the alarm or timer. |
| `args.snooze_seconds` | number \| null | no |  |
| `args.title` | string | no |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.seconds` | integer \| null | no |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.latitude` | number | yes |  |
| `args.longitude` | number | yes |  |
| `args.name` | string | yes |  |
| `args.radius` | number | yes | Metres. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.name` | string | yes |  |
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
| `args.activity` | string | no | Required for a workout. One of: `walking`, `running`, `cycling`, `other` |
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.kind` | string | yes | One of: `water`, `weight`, `mindful`, `workout` |
| `args.start` | string | yes |  |
| `args.tag` | string | yes |  |
| `args.value` | number | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.progress` | number | yes |  |
| `args.status` | string | yes |  |
| `args.task_id` | string | yes | Your id for this activity. Not _start. |
| `args.title` | string | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.progress` | number | yes |  |
| `args.status` | string | yes |  |
| `args.task_id` | string | yes | Your id for this activity. Not _start. |
| `args.title` | string | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.progress` | number | yes |  |
| `args.status` | string | yes |  |
| `args.task_id` | string | yes | Your id for this activity. Not _start. |
| `args.title` | string | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.enabled` | boolean | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.days` | integer | yes |  |
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.body` | string | yes |  |
| `args.buttons` | array<object> | no |  |
| `args.buttons[].id` | string | yes | Not starting with com.apple. |
| `args.buttons[].title` | string | yes |  |
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.thread` | string | no |  |
| `args.time_sensitive` | boolean | no |  |
| `args.title` | string | yes |  |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.end_minute` | integer | yes |  |
| `args.start_minute` | integer | yes | Minute of the day. The window must be at least 15 minutes. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.id` | string | yes | UUID of the alarm or timer. |
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
| `args.device_id` | string | yes | The paired iPhone, as iphone:<id>. See iphone.devices. |
| `args.duration_seconds` | number | yes |  |
| `args.id` | string | no | UUID of the alarm or timer. |
| `args.title` | string | no |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | no |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## location

### location.current

Read the device location. An agent the owner set to ask first gets {id,state:awaiting_approval} and, after owner approval, retries the identical request ID once; that approval expires after ten minutes and is consumed even if the read fails. Coordinates are never stored in approval records.

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.current` |

### location.geocode

Geocode an explicit address; never requests or returns the device current location

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.geocode` |

### location.reverse_geocode

Reverse geocode explicit coordinates; never requests or returns the device current location

- Kind: `query`
- Scope: `location:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | value | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |
| `op` | string | no | One of: `location.reverse_geocode` |

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
| `args.body` | string | no |  |
| `args.cc` | array<string> | no |  |
| `args.subject` | string | yes |  |
| `args.to` | array<string> | yes |  |
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
| `args.body` | string | no |  |
| `args.cc` | array<string> | no |  |
| `args.subject` | string | yes |  |
| `args.to` | array<string> | yes |  |
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
| `args.account` | string | yes |  |
| `args.id` | string | yes |  |
| `args.mailbox` | string | yes |  |
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
| `args.flagged` | boolean | yes |  |
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
| `args.read` | boolean | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.from` | object | no |  |
| `args.from.lat` | number | yes |  |
| `args.from.lon` | number | yes |  |
| `args.lat` | number | no |  |
| `args.lon` | number | no |  |
| `args.mode` | string | no | One of: `driving`, `walking`, `transit`, `cycling` |
| `args.to` | object | no |  |
| `args.to.lat` | number | yes |  |
| `args.to.lon` | number | yes |  |
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
| `args.from` | object | yes |  |
| `args.from.lat` | number | yes |  |
| `args.from.lon` | number | yes |  |
| `args.mode` | string | yes | One of: `driving`, `walking`, `transit`, `cycling` |
| `args.to` | object | yes |  |
| `args.to.lat` | number | yes |  |
| `args.to.lon` | number | yes |  |
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
| `args.chat_id` | string | no |  |
| `args.files` | array<object> | no |  |
| `args.files[].data` | string | no |  |
| `args.files[].name` | string | yes |  |
| `args.files[].upload_id` | string | no |  |
| `args.handles` | array<string> | no |  |
| `args.resend_of` | string | no |  |
| `args.service` | string | no | One of: `iMessage`, `SMS`, `RCS` |
| `args.text` | string | no |  |
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
| `args.files` | array<object> | no |  |
| `args.files[].data` | string | no |  |
| `args.files[].name` | string | yes |  |
| `args.files[].upload_id` | string | no |  |
| `args.id` | string | yes |  |
| `args.part_index` | integer | no |  |
| `args.text` | string | yes |  |
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
| `args.emoji` | string | no |  |
| `args.id` | string | yes |  |
| `args.part_index` | integer | no |  |
| `args.reaction` | string | yes | One of: `love`, `like`, `dislike`, `laugh`, `emphasize`, `question`, `emoji` |
| `args.remove` | boolean | no |  |
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
| `args.id` | string | yes |  |
| `args.part_index` | integer | no |  |
| `args.text` | string | yes |  |
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
| `args.id` | string | yes |  |
| `args.part_index` | integer | no |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.folder_id` | string | yes |  |
| `args.plain_text` | string \| null | no |  |
| `args.title` | string \| null | yes |  |
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
| `args.id` | string | yes |  |
| `args.mode` | string | no | One of: `replace`, `append` |
| `args.plain_text` | string \| null | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.title` | string \| null | yes |  |
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
| `args.folder_id` | string | yes |  |
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
| `args.title` | string | yes | Only test folders can be renamed: the title must start with "Bridge Test [ab-test". |
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
| `args.annotations` | array<object> | yes |  |
| `args.annotations[].height` | number | yes |  |
| `args.annotations[].page` | integer | yes |  |
| `args.annotations[].text` | string | no |  |
| `args.annotations[].type` | string | yes | One of: `note`, `highlight` |
| `args.annotations[].width` | number | yes |  |
| `args.annotations[].x` | number | yes |  |
| `args.annotations[].y` | number | yes |  |
| `args.blob` | object | no | A blob the caller may already read: query_id with digest, or collection and record_id with digest |
| `args.blob.collection` | string | no |  |
| `args.blob.digest` | string | yes |  |
| `args.blob.query_id` | string | no |  |
| `args.blob.record_id` | string | no |  |
| `args.data_base64` | string | no |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

## photos

### photos.search

Live PhotoKit search with bounded fetches. Date/type/album filters run in PhotoKit; coordinate-radius filtering examines at most 1000 candidates per page. examined_count is this page's candidates, not a library total. Follow next_cursor, even on empty pages, keeping the same filters. Each asset has place, its place name, when it has a location. No library metadata is stored. Pages are live, not snapshots.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.album_id` | string | no |  |
| `args.cursor` | string | no |  |
| `args.end` | string | no |  |
| `args.favorite` | boolean | no |  |
| `args.include_hidden` | boolean | no |  |
| `args.limit` | integer | no |  |
| `args.near` | object | no |  |
| `args.near.lat` | number | yes |  |
| `args.near.lon` | number | yes |  |
| `args.radius_m` | number | no |  |
| `args.start` | string | no |  |
| `args.type` | string | no | One of: `image`, `video`, `audio`, `live_photo`, `screenshot`, `panorama`, `hdr`, `depth`, `slow_motion`, `timelapse` |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.places

Where photos were taken, by place name: each place (city, US state, country) with photo_count, first_taken_at, last_taken_at and one representative asset_id, plus countries and us_states rollups. Scans the library live between start and end, skipping screenshots unless include_screenshots. Names come from Apple's geocoder and are remembered only in memory, per 5 km cell; when complete is false, Apple's rate limit stopped the lookup early, so call again for the rest. No library metadata is stored.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.end` | string | no |  |
| `args.include_hidden` | boolean | no |  |
| `args.include_screenshots` | boolean | no |  |
| `args.start` | string | no |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.get

Live asset details, location and all public PhotoKit resources, including video, paired Live Photo video, adjustment data and edited/original representations. Resource indexes are local to this response.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.asset_id` | string | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.albums

Page through user albums or smart albums, with PhotoKit editing capabilities. No album index is stored.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.limit` | integer | no |  |
| `args.offset` | integer | no |  |
| `args.smart` | boolean | no |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.preview

Look at one photo. Renders a JPEG preview on demand, longest side max_px (64-1536, default 512), and returns it as an image the agent can view; REST callers get image_base64. Video assets return a still frame. Hidden assets need include_hidden. network_access (default true) lets PhotoKit download this one original from the owner's iCloud when the Mac keeps only a thumbnail; set false to use local data only. Each call sends this photo's pixels to the caller; nothing is cached or sent ahead of time. Narrow with photos.search first and preview only what you need.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.asset_id` | string | yes |  |
| `args.include_hidden` | boolean | no |  |
| `args.max_px` | integer | no |  |
| `args.network_access` | boolean | no |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.contact_sheet

Scan many photos at once. Renders up to 30 asset_ids from photos.search into one JPEG grid of numbered thumbnails (columns 1-8, default up to 5; cell_px 96-384, default 256) and returns it as an image the agent can view. cells maps each number to its asset_id; open a match larger with photos.preview. Hidden assets need include_hidden; network_access (default false) allows iCloud downloads, and cells that are not local otherwise show grey with rendered false. Pixels leave the Mac only for the photos named in this call.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.asset_ids` | array<string> | yes |  |
| `args.cell_px` | integer | no |  |
| `args.columns` | integer | no |  |
| `args.include_hidden` | boolean | no |  |
| `args.network_access` | boolean | no |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.fetch

Start a temporary original-resource transfer with asset_id, optional resource_index, resource_key from photos.get to detect changed metadata, and network_access (default false, allows iCloud downloads). rendition display instead transfers the photo as Photos shows it, edits included, as a full-resolution JPEG with no metadata and longest side at most 8000 px, re-encoded or scaled only as far as needed to stay within max_bytes; video assets give a still frame. Poll/read with transfer_id/offset/length only. Returns downloading or ready with SHA-256, size and base64 bytes. Read at most 512 KiB per call. Default cap 8 GiB, configurable up to 64 GiB, subject to disk space. Four transfers; poll within 120 seconds to retain one, with a 24-hour hard lifetime. Release explicitly. Never put photo bytes into Bridge blobs or replication.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.asset_id` | string | no |  |
| `args.length` | integer | no |  |
| `args.max_bytes` | integer | no |  |
| `args.network_access` | boolean | no |  |
| `args.offset` | integer | no |  |
| `args.rendition` | string | no | One of: `display` |
| `args.resource_index` | integer | no |  |
| `args.resource_key` | string | no |  |
| `args.transfer_id` | string | no |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.release

Cancel and delete this principal's temporary Photos transfer. Expired or foreign transfer IDs fail closed.

- Kind: `query`
- Scope: `photos:read`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.transfer_id` | string | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | no |  |

### photos.favorite

Set an asset's favorite flag and read it back. Transient write that runs at once unless the owner set this agent to ask first; then it returns awaiting_approval and the identical request ID is retried once after approval. Completed or ambiguous IDs never redispatch. Only a request digest, not asset details, is retained.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.asset_id` | string | yes |  |
| `args.favorite` | boolean | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

### photos.album_create

Create a regular user album. Transient write. An agent set to ask first gets a hash-only approval and reissues the identical ID after it. No automatic retries.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.title` | string | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

### photos.album_rename

Rename an editable regular album. Transient write; asks the owner first only for an agent set to ask first. No automatic retry.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.album_id` | string | yes |  |
| `args.title` | string | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

### photos.album_add

Add up to 200 existing assets to an editable regular album. Does not import or copy media. Transient write; asks the owner first only for an agent set to ask first.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.album_id` | string | yes |  |
| `args.asset_ids` | array<string> | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

### photos.album_remove

Remove up to 200 memberships from an editable regular album, never from the library. Transient write; asks the owner first only for an agent set to ask first.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.album_id` | string | yes |  |
| `args.asset_ids` | array<string> | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

### photos.album_delete

Delete an editable regular album, never its assets. Transient write; asks the owner first only for an agent set to ask first. Asset deletion and permanent deletion are unavailable.

- Kind: `query`
- Scope: `photos:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `args` | object | yes |  |
| `args.album_id` | string | yes |  |
| `deadline_ms` | integer | no |  |
| `id` | string | yes |  |

## print

### print.document

Print an authorized blob or supplied PDF bytes to PDF or a named printer

- Kind: `command`
- Scope: `print:write`

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `admission_era` | string | no |  |
| `args` | object | yes |  |
| `args.blob` | object | no | A blob the caller may already read: query_id with digest, or collection and record_id with digest |
| `args.blob.collection` | string | no |  |
| `args.blob.digest` | string | yes |  |
| `args.blob.query_id` | string | no |  |
| `args.blob.record_id` | string | no |  |
| `args.data_base64` | string | no |  |
| `args.printer` | string | yes |  |
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
| `args.alarms` | array<object> \| null | no |  |
| `args.alarms[].absolute_date` | string | no |  |
| `args.alarms[].latitude` | number \| null | no |  |
| `args.alarms[].location_title` | string | no |  |
| `args.alarms[].longitude` | number \| null | no |  |
| `args.alarms[].proximity` | string | no | One of: `arrive`, `leave` |
| `args.alarms[].radius_m` | number \| null | no |  |
| `args.alarms[].relative_offset` | number \| null | no |  |
| `args.due` | string \| null | no |  |
| `args.due_all_day` | boolean \| null | no |  |
| `args.list_id` | string | yes |  |
| `args.notes` | string \| null | no |  |
| `args.priority` | integer \| null | no |  |
| `args.start` | string \| null | no |  |
| `args.title` | string \| null | yes |  |
| `args.url` | string \| null | no |  |
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
| `args.title` | string \| null | yes |  |
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
| `args.alarms` | array<object> \| null | no |  |
| `args.alarms[].absolute_date` | string | no |  |
| `args.alarms[].latitude` | number \| null | no |  |
| `args.alarms[].location_title` | string | no |  |
| `args.alarms[].longitude` | number \| null | no |  |
| `args.alarms[].proximity` | string | no | One of: `arrive`, `leave` |
| `args.alarms[].radius_m` | number \| null | no |  |
| `args.alarms[].relative_offset` | number \| null | no |  |
| `args.due` | string \| null | no |  |
| `args.due_all_day` | boolean \| null | no |  |
| `args.id` | string | yes |  |
| `args.list_id` | string | no |  |
| `args.notes` | string \| null | no |  |
| `args.priority` | integer \| null | no |  |
| `args.start` | string \| null | no |  |
| `args.title` | string \| null | no |  |
| `args.url` | string \| null | no |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
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
| `args.id` | string | yes |  |
| `args.title` | string \| null | yes |  |
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
| `args.id` | string | yes |  |
| `command_id` | string | no | Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only. |
| `expected_version` | integer | yes |  |
| `expires_at` | string | no |  |
| `op` | string | no |  |
| `origin` | string | no |  |
| `wait_ms` | integer | no |  |

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
| `args.text` | string | yes |  |
| `args.volume` | number | no |  |
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

