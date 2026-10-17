# One deep State module

Records, aliases, journal, checkpoints and command transitions live in one SQLite-backed module so that a capture and its checkpoint, or a command transition and its journal row, commit atomically. Splitting these into repositories would spread atomicity and recovery logic across callers. The command ledger is retained independently of journal retention and mirror rebuilds.
