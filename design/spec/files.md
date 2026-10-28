# Files

Files captures metadata only from iCloud Drive/Bridge Test and additional absolute roots listed in Library/Application Support/AppleBridge/files-roots.json. At most sixteen additional roots, ten thousand items per root and sixteen directory levels are admitted. Symlinks are excluded. Incomplete scans never establish absence. Captured metadata excludes file contents.

files.search accepts query and field (name, content, kind, date, tag) and runs Spotlight within opted-in roots. Results are bounded to one hundred paths. files.read accepts root and relative path, confines the resolved path to an opted-in root, starts iCloud hydration and stages a digest blob. Queries use the transient query contract.

Real Spotlight content search passed. The single offload run failed the native eviction capability guard; real eviction and hydration remain unverified. Five-minute standalone source idle CPU measured 0.06277591117841626%; host contention prevents quiet qualification and full Helper CPU remains unverified. Arbitrary Mac-wide search is not exposed.

File commands accept root plus a relative path. Create and write accept parent, name, content_base64 and a SHA-256 digest (one MiB maximum). Move and rename accept path, parent and name. Tags accept path and tags. Trash confirms the returned Trash location exists. Download and evict require an ubiquitous item and poll downloaded state for up to ten seconds. Prepared actions are revalidated before dispatch. Readback failures remain indeterminate. Command completion reconciles metadata through the normal capture sink; reconciliations are serialized with source capture. Absolute paths, dot traversal and symlink components are rejected.

FILE-COMMANDS exercises the native source in a temporary home; FILE-CRUD-REAL confines fixtures to a uniquely named child of Bridge Test. FILE-REPLICA fails explicitly until the Ubuntu gateway journey is implemented. Simapple command coverage and concurrent hostile filesystem mutation qualification remain outstanding.
