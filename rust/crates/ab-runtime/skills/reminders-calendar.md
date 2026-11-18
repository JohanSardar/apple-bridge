# Apple Bridge: reminders and calendar

Times are RFC 3339 timestamps with an offset, such as 2026-11-12T09:00:00-06:00, or local times without one, such as 2026-11-12T09:00, which mean the Mac's time zone. Plain dates YYYY-MM-DD mean midnight there. Results add `start_local`, `end_local` and `due_local`.

## Reminders

- `reminders_add` with `title`, and optionally `list` (a name or lst_ ID), `due`, `notes` and `priority`. A plain date makes an all-day reminder.
- Read with `apple_read` and `reminders.items`, choosing one of `smart` (today, scheduled, flagged, all, completed), `list` or `tag`. `reminders.lists` lists the lists. `smart: today` returns `due_today`, `overdue` (count and the latest three) and `upcoming` (the next seven days); pass `view` with one of those names for that group in full.
- Complete with `apple_change` and `reminders.complete`, passing `reminder` with its title, or `id` with the reminder's `version` as `expected_version`. `reminders.uncomplete`, `reminders.update` and `reminders.delete` work the same way; `reminders.create_list` makes a list.

## Calendar

- `calendar_events` lists today's events by default, including all-day and multi-day events that cover the day. Use `date` for one day, or `start` and `end` for a range, `calendar` (a name such as Work) and `text` to filter. Events come as short cards; `detail: true` adds notes, attendees and alarms.
- `calendar_add` with `title` and `start`, and optionally `end` (default one hour later), `calendar` (a name; default the first calendar that accepts events), `location` and `notes`. A plain date makes an all-day event. For alarms, recurrence or availability use `apple_change` and `calendar.create_event`, which takes `calendar` or `calendar_id`. `apple_read` with operation calendar.calendars lists the calendars.
- Change with `calendar.update_event` or `calendar.delete_event`, passing `expected_version`. For repeating events, `span` chooses this occurrence or all future ones.
