# Notes extras

`notes.move` accepts `id`, `folder_id`, and `expected_version`. Both current and destination folders pass the existing writable-ancestor checks. The Helper uses Notes Automation and reports success only after NoteStore shows the destination folder. SimApple implements the same command and capture evidence.

Existing reads expose checklist block state, optional pins, and decoded hashtag attachments. Notes.sdef on macOS 26 exposes no checklist, tag, or pinned setter. HTML replacement cannot preserve native checklist identities; these writes remain unavailable.

Folder creation uses the existing command. Folder deletion remains subject to the existing per-build deletion proof. Tagged test folders can be renamed with `notes.rename_folder`, guarded by expected version and the Bridge Test tag prefix. `NOTE-EXTRAS` exercises folder creation, rename and move through REST and MCP, compares the result with Notes Automation, and removes only the captured fixtures and folders from its run.

Native note deletion, move and rename use public Notes Automation commands and verify the resulting NoteStore state before reporting success. Prepared folder state retains only a stable digest. Note fingerprints cover content, lock, pin, folder and attachment dependencies; unrelated cloud bookkeeping does not invalidate a prepared change.

Deleting a note can leave its row in NoteStore with `ZMARKEDFORDELETION=0` while moving it to a folder with `ZFOLDERTYPE=1`. Reads exclude notes marked for deletion and notes under trashed or marked folders. Incremental capture emits an absence for those notes, including when an ancestor folder moves into trash; restoring a note makes it readable again.

Cleanup baselines use a read-only SQLite identity scan without fetching every note body through Automation. Tagged body comparisons select only the current run's notes before reading their content through Notes. Cleanup requires the exact captured ID, run tag and owned folder, deletes through the validated container-scoped Automation object, verifies the deletion state, and checks that the active Notes baseline is restored. Empty run-created folders are validated by captured ID and exact title as one batch before deletion, then verified together. A delayed check is followed by read-only verification of those same IDs. Existing notes and older test fixtures are preserved.
