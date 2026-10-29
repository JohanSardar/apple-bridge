# Helper idle scheduling

Permission failures use the capture retry backoff (2, 4, 8, 16, 30 seconds), including sources whose permission is absent. A capture that ran for at least 30 seconds resets the retry delay. Readiness is read once per attempt and reused for the failure state. EventKit authorization and session readiness polling double from one second to at most 30 seconds while unchanged and reset when status changes. All checks use non-prompting status APIs; command prepare and dispatch retain their fresh permission checks.

The Helper observes gateway process exit with a dispatch process source. It no longer writes a lifetime trace every second. Trace output lazily retains one append-only, close-on-exec descriptor per trace, serialized by its existing lock; event timestamps and digests are unchanged.

## Timer audit

Production timers and polling in `native/Sources` were inspected. Short sleeps in command readback, hydration, foreground coordination, exports, benchmarks, checks and lab fixture code require active work and do not run during a settled idle capture.

| Idle path | Scheduling and freshness |
| --- | --- |
| Usage | Recursive filesystem events and power notifications; unchanged fallback doubles from 500 ms to 30 seconds. See usage.md. |
| Safari | Five-second tab cadence retained; denied Automation cached for 30 seconds, allowed status rechecked before AppleScript. Database/WAL events plus 30-second fallback skip unchanged inputs; rolling history expiry retained. |
| Location / Maps | Status-only capture returns every 30 seconds; Location reuses a manager without starting updates. Queries still require a fresh fix. |
| Additional source failures | Exponential retry capped at 30 seconds, including missing permission. |
| EventKit authorization / session readiness | Exponential polling capped at 30 seconds; notifications and command checks remain independent. |
| Calendar / Reminders | EventKit and database notifications, calendar day-boundary check each second, 15-minute verification retained for date-window and missed-event correctness. |
| Messages | Two-second watcher fallback and wake notifications retained; unchanged database versions skip reconciliation. Settled media worker waits for input; active power/thermal retry is one second. |
| Contacts | Store notifications and 30-second history-token fallback retained. |
| Notes | Filesystem/WAL notifications and five-minute reconciliation fallback retained. |
| Files / shared DatabaseWatcher | Filesystem notifications and 30-second recovery fallback retained for replacement and missed notifications. |
| Photos / Shortcuts / Focus | Existing 30-second capture and change suppression retained. |
| Music | Existing 60-second library comparison retained; one-second publishing wait runs only during active publication. |
| Session / Helper | Wire credit and frame waits remain event-driven; reconnect backoff only runs while disconnected. The one-second main run-loop limit remains for session completion; gateway lifetime uses a process-exit event. |
