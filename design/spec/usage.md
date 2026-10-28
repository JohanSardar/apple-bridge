# Usage input fingerprints

Usage keeps its 500 ms fallback and filesystem notifications. Fingerprints retain regular file paths, sizes and modification dates, sync database and WAL metadata, power-event count and time zone. Immutable URL metadata key sets are allocated once per fingerprint rather than for every file. Append, remote arrival, sync metadata, power and time-zone changes still invalidate the fingerprint.
