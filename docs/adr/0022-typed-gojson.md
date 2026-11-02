# Preserve legacy JSON with typed gojson entry points

Status: Accepted for the Rust migration.

ab-wire owns gojson entry points for CommitCapture record data (UseNumber and Go marshaling), CommandHash ordered capitalized keys, query no-HTML mode, RFC3339Nano timestamps and raw signed bytes. Generic canonical JSON cannot replace these paths. Preserve number lexemes, map order rules, escaping and struct field order. Swift source generations need SwiftDescription compatibility or an explicitly tested rescan.

Ban serde_json arbitrary_precision and preserve_order workspace-wide. Require zero unexplained byte mismatches over synthetic goldens, raw capture frames and a read-only live state clone before cutover. Golden tests are explicit compatibility/decoder gates authorized by Phase 0, not general implementation unit tests.
