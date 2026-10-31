# Screen Time action qualification

The pack `AB Screen Time` is not supplied or imported yet. No executable action identifier has been qualified on this Mac; do not synthesize one or run an owner's existing shortcut as a substitute.

F9 follow-up inspected the Air's macOS 26.7.1 builtin action definitions and installed system application App Intent metadata. Neither exposed “Get App & Website Data.” The repository workflow builder can serialize verified actions, but cannot make an unavailable Apple action executable. Creating a text-only placeholder would not probe Screen Time. Keep this real limit until a supported action or a qualified owner export is available.

On a system that exposes Apple's “Get App & Website Data” action, create a dedicated `AB Screen Time` shortcut and approve its access. Export the qualified shortcut to `packaging/shortcuts/AB Screen Time.shortcut` before running the probe. Keep raw results temporary and local. Evidence must contain only counts and coverage, never app names, websites, usage durations or device identifiers.

Compare identical date windows against Biome: device count and platform coverage, app and website row counts, missing fields and availability. Retain Biome until output shape, privacy, device reach and unattended execution are verified. Inventory alone never qualifies a shortcut to execute.

`USE-ACTION-PROBE` now uses a bounded runner after exactly one dedicated pack is installed and its qualified signed export exists at the path above. It requires nonempty output, keeps only its byte count and SHA-256 digest in evidence, and never opens an import or permission dialog. This proves probe execution only; it does not qualify replacing Biome. Its simulation uses synthetic output and does not access Shortcuts or usage data.
