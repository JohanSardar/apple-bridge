The Calendar records are the calendar descriptor and first event from
`lab/scenarios/calendar_store_perf.go:calendarRangePerf` (2026-05-01 UTC).
The Contacts records are the contact and group entries from
`lab/scenarios/contacts_list.go`.

`NativeChecks` pages these exact synthetic facts through fake adapters and checks
that fields and native references survive unchanged. It never opens EventKit,
Contacts, or a production Helper. Run from the repository root:

```
native/.build/clt-release/NativeChecks lab/fixtures/native-1/facts.json
```
