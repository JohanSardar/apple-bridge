# Usage input fingerprints

Usage uses recursive filesystem notifications with 100 ms coalescing for nested local segments, remote arrivals and sync metadata. Its fallback starts at 500 ms, doubles while fingerprints are unchanged, and caps at 30 seconds; a changed fingerprint resets it to 500 ms. Power notifications still trigger immediate checks. Time-zone changes and missed filesystem events are detected within the 30-second fallback bound. Fingerprints retain regular file paths, sizes and modification dates, sync database and WAL metadata, power-event count and time zone. A cached inventory checks POSIX file and directory identities, sizes, modification and change times before rebuilding the recursive URL metadata fingerprint. Directory stamps detect additions and removals; missing sync database and WAL paths remain tracked. Append, remote arrival, sync metadata, power and time-zone changes still invalidate the fingerprint.
# Usage action probe

Biome remains the usage source. `USE-ACTION-PROBE` captures and replicates Usage in an isolated world and compares it with a separately decoded Biome snapshot. It never runs a shortcut. The dedicated Screen Time pack is absent; independent Screen Time parity remains unproven. See [comparison and adoption decision](../../research/screentime-probe-results.md) and [pack setup](../../packaging/shortcuts/screen-time-probe.md).

# Focus attribution and device identity

Names use nonempty DevicePeer metadata, then the local host name for the local device. Missing remote names use platform plus an eight-character device identifier and `name_status=platform_id_fallback`; no owner name or hardware model is inferred from an OS build string.

Focus transitions are deduplicated and serialized per device. Known Universal Control, Notification Center, Control Center and System Settings agents remain as `usage_class=excluded_system_ui` rows with zero credited seconds. Their bounded intervals contribute to `clipped_seconds`. Local knowledgeC lock/display-off intervals are corroborating clipping evidence, never added usage; only sourceless local rows or an exact hardware UUID match qualify. Overlapping clipping intervals are subtracted once. Missing or unreadable historical evidence remains an explicit coverage gap. The Helper's existing power notifications clip new local sleep intervals. Remote sleep remains unproven where the focus stream lacks lock evidence.

Daily attribution is limited to 86,400 seconds, including a 25-hour daylight-saving day. The excess is clipped, not scaled. Four-hour session caps and omission of unclosed sessions remain. These are foreground estimates rather than proof of active interaction; long unattended focus can remain where no lock/display evidence exists.

The real probe compares corrected rows with a separately decoded raw-focus upper bound and checks conservation of credited plus clipped duration. This does not establish independent Screen Time parity. Optional `APPLE_BRIDGE_USAGE_PROOF_DIR` exports only device labels, days, app identifiers and duration counts from the isolated world. Independent knowledgeC app-usage comparisons must disclose coverage and deltas separately.
