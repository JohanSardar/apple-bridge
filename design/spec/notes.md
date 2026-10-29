# Notes extras

`notes.move` accepts `id`, `folder_id`, and `expected_version`. Both current and destination folders pass the existing writable-ancestor checks. The Helper uses Notes Automation and reports success only after NoteStore shows the destination folder. SimApple implements the same command and capture evidence.

Existing reads expose checklist block state, optional pins, and decoded hashtag attachments. Notes.sdef on macOS 26 exposes no checklist, tag, or pinned setter. HTML replacement cannot preserve native checklist identities; these writes remain unavailable.

Folder creation uses the existing command. Folder deletion remains subject to the existing per-build deletion proof. Tagged test folders can be renamed with `notes.rename_folder`, guarded by expected version and the Bridge Test tag prefix. Real rename and move qualification remain pending a verified cleanup route: existing real evidence reports iCloud rows surviving AppleScript deletion. NOTE-EXTRAS therefore performs no real mutation and explicitly reports limited coverage.

Native move and rename capabilities require `notes-extras-proofs.json` in the Helper home: version 1, matching `os_build` and source `generation`, and per-operation `before_active`, `command_completed`, `notestore_verified`, `after_matches`, and `cleanup_complete` evidence. No qualification ledger is produced while real cleanup is unverified; these native capabilities remain unadvertised and reject `capability_unproven`. Prepared folder state retains only a stable digest.
