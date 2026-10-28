# Phone call capture

`phone.call` reads the local CallHistory Core Data store through the shared WAL-aware read-only snapshot. Keys are generation-scoped Z_PK values. The WAL watcher reconciles duration and answered state. Only completed scans authorize absence. Unknown schemas and service providers fail closed. Notifications are filtered by database/WAL identity, size and nanosecond modification/change times, so SQLite shared-memory read marks do not cause rescans. No calls are placed and no database writes are made.

Fields: direction (outgoing/incoming/missed), handle, name, date (UTC), duration (seconds), service (cellular/FaceTime audio/FaceTime video), answered (boolean). Grants: phone:read and phone:write; no write operation is advertised. Doctor includes call shape violations. Locally synced history is the coverage boundary.

Generated fixtures contain synthetic data only. Evidence contains assertions, counts and digests only. The independent Truth probe compares thirty-day counts and record digests in memory; only counts and assertions reach evidence. Five-minute source and signed Helper idle measurements enforce a two-percent CPU ceiling.

Real idle qualification is limited: the full Helper averaged 48.29% CPU over five minutes on a quiet host, with Messages reconciliation active in a sample. The phone fixture passes its five-minute CPU and unchanged-checkpoint guards.

Continuation qualification: native build, capture fixture and Ubuntu replica pass. The fresh real-read and real-idle attempts exhausted their shared signed Helper build windows; descendant build processes retained output pipes after timeout and were cleaned up. Earlier real-read evidence matched 117 record digests; fresh full-Helper qualification is limited by the harness. Phone scenario budgets allow 45 minutes for shared build contention.

The freshly compiled CallsChecks idle guard passes five minutes below 2% CPU with unchanged checkpoints. Its redundant scenario rebuild was cancelled after 38 minutes waiting for the shared Swift lock; the earlier full sim scenario passed.
