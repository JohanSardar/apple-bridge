# Usage input fingerprints

Usage keeps its 500 ms fallback and filesystem notifications. Fingerprints retain regular file paths, sizes and modification dates, sync database and WAL metadata, power-event count and time zone. Immutable URL metadata key sets are allocated once per fingerprint rather than for every file. Append, remote arrival, sync metadata, power and time-zone changes still invalidate the fingerprint.
# Usage action probe

Biome remains the usage source. `USE-ACTION-PROBE` is a read-only real inventory check: it counts installed shortcuts and immediate local/remote Biome entries, retaining no owner shortcut names or usage values. It never runs any shortcut. The dedicated Screen Time pack is absent; action output availability and row coverage remain limited. See [comparison and adoption decision](../../research/screentime-probe-results.md) and [pack setup](../../packaging/shortcuts/screen-time-probe.md).
