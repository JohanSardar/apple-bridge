# Search index maintenance

Search index version 3 assigns each `(collection, id)` a persistent integer in
`records_fts_map`. Search triggers use that integer as the FTS5 rowid for inserts
and deletes. Updating a record no longer searches the unindexed identity columns
of every FTS row. The mapping uses an `INTEGER PRIMARY KEY`, so its value survives
VACUUM independently of the implicit rowid in `records`.

Existing populated stores rebuild the index in the background. Startup installs
triggers targeting the replacement index before allowing captures. The rebuild
visits at most 512 records per write transaction and releases the writer between
batches. Captures update the replacement index in the same transaction as their
records. The final transaction swaps the indexes and records `fts_version=3`.

Text queries return retryable `unavailable` until the replacement index is complete.
The REST search operation returns HTTP 503 with the typed error. The MCP search
tool returns `isError: true` and the same typed error in its structured and text
content, rather than a successful empty result. The web shell currently contains
an unconnected Search field; browser search results and retry presentation are
not implemented or certified by this migration.
Ordinary reads and captures remain available. Closing the store cancels and joins
the worker; reopening resumes an interrupted rebuild using the same mapping.
A new empty store starts with version 3 immediately.

Text queries apply source visibility inside the FTS candidate query as well as
the outer records query. A metadata-only principal therefore avoids building a
temporary identity set containing every matching hidden message. Per-record
visibility checks, including command ownership, remain enforced after lookup.

SQLite documents [persistent INTEGER PRIMARY KEY values](https://sqlite.org/rowidtable.html)
and [FTS5 integer row identities](https://sqlite.org/fts5.html).
