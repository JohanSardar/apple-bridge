# Messages reconciliation

The live cursor drains new ROWIDs in batches of 128 and rechecks recent messages, referenced messages, recoverable membership and small collections when SQLite's data version changes. Referenced messages and absence checks use bounded batch lookups.

History reconciliation captures the database generation, connection-local data version and maximum ROWID. A completed unchanged database skips history work, including after wake notifications and the fifteen-minute fallback timer. Reopening a connection invalidates the version checkpoint, so WAL rotation cannot reuse a connection-local version from another connection. Database replacement invalidates the generation and all capture fingerprints.

History batches materialize chat, recovery and attachment joins only for the selected ROWIDs and use their projected rows directly, yield between batches and retain native row fingerprints to suppress unchanged observations. Older edits and deletions still receive a conservative history audit after database changes; max ROWID alone cannot detect them. Only successful complete scans advance the reconciliation checkpoint. Concurrent live observations keep their newer timestamps, and missing message identities are revalidated in batches before publishing absence.

Wake reconciliation requests are consumed before boolean short-circuiting. File hints are coalesced in fixed 100 ms windows so continuous churn cannot postpone a wake indefinitely; shared-memory writes are not a data-change signal. SQLite data version remains authoritative. Polling writes watcher diagnostics once per tick.

The media drain loop shares the existing one-second power and thermal cache with active media checkpoints. It checks for ready jobs before consulting power state and does not query IOKit on each scheduling check. Lab pause files remain checked directly.
Empty blob and media workers wait 100 ms between checks; active work and capture notifications retain their existing scheduling.

Messages scenarios that invoke native fixture checks allow up to forty-five minutes for the serialized native build and assertions. Their latency assertions retain their existing budgets.

Lab Helpers publish a content-free idle marker after the initial complete scan and ready blob/media work drain. Media waiting for a scheduled retry does not block the marker; the measurement includes any retries that become ready during its window. MAC-IDLE waits up to thirty minutes for that explicit startup barrier, then measures whole-Helper CPU for five minutes. The real Mac scenario budget includes automatic signed-Helper builds as well as the startup barrier and measurement. Startup timeout fails the check; no CPU samples are discarded during its measurement window.

A failed CPU check can save a private stack sample after its timed window. Its CPU result and duration are fixed before profiling begins.

Sources that repeatedly fail capture use exponential retry delays from two to thirty seconds. Explicit permission-missing capture errors retain one-second recovery. Readiness labels alone do not bypass backoff because a source may use the same label for unavailable or unsupported stores. A capture that ran at least thirty seconds or returned successfully resets the backoff. Lab traces record only the source and chosen delay.

The session run loop uses a one-second run limit rather than waking itself every 50 ms. Native sources and dispatch callbacks continue to run inside that interval; the limit bounds checking session completion. A controlled idle-loop benchmark measured 0.231 percent CPU at 50 ms and 0.016 percent at one second. With that run limit, a 100 ms MainActor timer fired after 109.5 ms, before the run limit expired.

The startup budget covers large real stores on the shared 8 GB Air. A busy-host run reached 147,880 of 168,210 message/reaction rows before its former fifteen-minute startup limit; it took no CPU window. Extending setup does not change the five-minute measurement or its two-percent threshold.
