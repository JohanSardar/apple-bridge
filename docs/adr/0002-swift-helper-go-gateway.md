# Swift helper for Apple I/O, Go for the shared gateway/replica program

A thin Swift Helper owns all Apple access (TCC permissions attach to it); one Go `bridge` binary in gateway or replica role owns State, replication, trust, REST and MCP. Go gives self-contained cross-compiled replicas for Linux and macOS with pure-Go SQLite (`modernc.org/sqlite`); a single Swift server would have tied replicas to Apple platforms.
