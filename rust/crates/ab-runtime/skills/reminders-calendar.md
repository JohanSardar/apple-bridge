# Apple Bridge: reminders and calendar

Dates are RFC 3339 timestamps with an offset, such as 2026-11-12T09:00:00-06:00, or plain dates YYYY-MM-DD where an argument says so. Without a time zone, the Mac's zone from `apple_status` is used.

## Reminders

- `reminders_add` with `title`, and optionally `list` (a name or lst_ ID), `due`, `notes` and `priority`. A plain date makes an all-day reminder.
- Read with `apple_read` and `reminders.items`, choosing one of `smart` (today, scheduled, flagged, all, completed), `list` or `tag`. `reminders.lists` lists the lists.
- Complete with `apple_change` and `reminders.complete`, passing `id` and the reminder's `version` as `expected_version`. Other changes: `reminders.update`, `reminders.delete`, `reminders.create_list`.

## Calendar

- `calendar_events` lists today's events by default. Use `date` for one day, or `start` and `end` for a range, and `text` to filter.
- Create with `apple_change` and `calendar.create_event`: `calendar_id`, `title`, `start`, `end`, and optionally `location`, `notes`, `all_day` and `recurrence`. `records.query` with collection calendar.calendar lists the calendars.
- Change with `calendar.update_event` or `calendar.delete_event`, passing `expected_version`. For repeating events, `span` chooses this occurrence or all future ones.
