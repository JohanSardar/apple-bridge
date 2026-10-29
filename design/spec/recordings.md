# Voice Memos and voicemail

The initial read-only slice captures recording and voicemail metadata from SQLite WAL snapshots. Each complete scan reconciles changed rows and confirmed absences. UUID native keys are preferred; fallback row keys are scoped to the database generation. Missing stores report unavailable with reason `store not present` before capture begins.

Voice Memos reads CloudRecordings.db; voicemail reads the macOS 26 FaceTime message store. No write operations are advertised. Both sources use the shared SourceSupport reader and complete-scan guard. Schema mismatches fail explicitly.

Coverage is limited to metadata. Folder records, embedded timed transcript decoding, voicemail archived transcripts, optional-column adaptation and on-demand audio hydration remain unimplemented. Transcript and audio availability are explicitly unavailable. No transcript or audio content enters lab evidence. Real reads cannot be qualified on this Mac because both stores are absent; a new-memo check needs Johan to record a memo. Whole Helper idle qualification depends on T53.
