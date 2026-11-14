# Signed shortcut builds (G0-6)

The Shortcuts source was removed ([ADR 0033](../../docs/adr/0033-remove-safari-shortcuts-and-mac-settings.md)): agents can no longer list shortcuts, run one by name or discover App Intents actions (`shortcuts.run`, `shortcuts.actions` and inventory capture are gone). What remains is the signed pack: building, signing and installing it, and the Focus and Clock shortcuts the gateway runs internally.

`cargo run --manifest-path rust/Cargo.toml -p ab-shortcuts -- build` compiles every `packaging/shortcuts/*.json` source into `.build/shortcuts/`, then signs each workflow using the system `/usr/bin/shortcuts sign`. Use `-source path.json` for one source, `-out directory` for another build directory, and `-unsigned` for portable compilation without signing. Signing defaults to `-mode auto`: try `anyone`, then `people-who-know-me`. Explicit modes disable fallback. The JSON stdout and `manifest.json` record the actual mode and unsigned SHA-256 for each file. A failed signer exits nonzero and never publishes its partial signed file.

`cargo run --manifest-path rust/Cargo.toml -p ab-shortcuts -- open ".build/shortcuts/[ab-test noop] Apple Bridge Signing Test.shortcut"` opens the signed file with macOS `open` for the user's import review. Build never opens or imports a workflow. CORE-SHORTCUT-SIGN never opens, imports or runs it.

## Source format

JSON version 1 has `name` and an ordered `actions` array. Each action has a unique `key`, a native Shortcuts action `identifier`, and a `parameters` object. Unknown top-level/action fields reject. Parameters preserve nested dictionaries, arrays, strings, numbers and booleans as native plist values; null rejects. Dictionary keys sort lexically, action order is preserved, and UUIDs derive from workflow name and action key. Generated `UUID` cannot be overridden. Identical sources compile to identical XML plists. Apple's signed archive can contain changing signing metadata; its bytes are not promised reproducible. Signatures remain Apple's responsibility.

```json
{
  "version": 1,
  "name": "Bridge text example",
  "actions": [
    {"key": "text", "identifier": "is.workflow.actions.gettext",
     "parameters": {"WFTextActionText": "Synthetic text"}},
    {"key": "child", "identifier": "is.workflow.actions.runworkflow",
     "parameters": {"WFWorkflowName": "Bridge child", "WFInput": {"$output": "text"}}}
  ]
}
```

`{"$output":"earlier-key"}` emits an action-output token attachment using that action's stable UUID. Unknown/forward output references reject. `{"$input":true}` emits a Shortcut Input attachment. `{"$group":"condition-name"}` emits a stable grouping UUID for native If/Otherwise/end actions. Reserved `$` keys reject malformed references. Output from the final action follows native Shortcuts behavior; native input/output settings belong in action parameters.


## Verification boundary

CORE-SHORTCUT-SIGN at sim compiles twice and compares bytes/digests, then independently parses the WFWorkflow plist with macOS `plutil`. At real it additionally signs the no-op source, checks AEA1 framing, and re-signs that signed file with the system CLI to prove Apple accepts the archive. Evidence records pass/fail, digests and signing mode only. No personal library content is read. There are no new runtime Sources or grants. Signed files are build artifacts, excluded under `.build/`.

## Runner adapter (T26, partial)

`internal/shortcut` invokes the CLI directly without shell expansion. Inventory uses `list --show-identifiers`, optionally `--folders` or `--folder-name`; malformed or duplicate identifiers reject the entire result. CLI names cannot establish input capabilities, so none are inferred. The native integration below registers capture and text dispatch.

Runs accept input bytes (text, URL text or file content) and return output bytes, process exit status and timeout status. Private temporary directories are removed after every result. Input and output are capped at 16 MiB. The default timeout is 30 seconds, bounded to five minutes; cancellation kills the CLI process group and reaps the CLI process. It does not promise to cancel downstream work that Shortcuts may already have handed to an app. Process success proves only runner completion, never arbitrary shortcut business effects. The adapter intentionally discards process stdout/stderr to avoid retaining personal diagnostics.

`bridge shortcuts install-pack [-pack directory]` validates all three signed AEA files before opening them for user review. It never clicks Add Shortcut. The required files are `AB Echo.shortcut`, `AB Clock.shortcut` and `AB Focus.shortcut`; only the `AB Echo` source is supplied in this partial change. Missing Clock/Focus files cause the installer to reject before opening anything. Coverage publishes the bundled protocol version; imported pack version qualification remains pending.

## Pack qualification

Installed pack qualification uses a unique `AB Pack v1` inventory marker and exactly one each of AB Echo, AB Clock and AB Focus from bounded `shortcuts list --show-identifiers`. Missing, duplicate or unavailable inventory omits `pack_version`; the bundled version alone never establishes installation. This marker establishes inventory presence, not workflow integrity. Import the marker alongside the signed pack.

## Helper status intent (T52)

`BridgeStatusIntent` provides the read-only “Apple Bridge status” App Intent and an App Shortcut phrase. It reports SMAppService registration state only; enabled does not prove gateway health. It does not register the service, request permissions, read personal data or run commands. Extracted metadata is checked against the source digest. `SIRI-INTENTS` requires execution of a dedicated qualified status shortcut and a recognized returned status before passing. Missing setup remains limited; metadata generation alone does not qualify execution.

F9 probes use dedicated, previously approved shortcuts and never import a pack or request a permission during a run. `AB Bridge Status` contains the Helper's **Apple Bridge status** action followed by Stop and Output referencing its result. `packaging/shortcuts/ab-bridge-status.json` targets the services-signed Helper (`com.johansardar.applebridge.services`) installed on the test Mac. The signed `packaging/shortcuts/AB Bridge Status.shortcut` is supplied for import; signing does not establish installation, action discovery or permission approval. The Siri check validates the metadata against the intent source and requires an actual recognized status result; generated metadata alone does not pass the real check.

`CLK-ALARM-REAL`, `CLK-TIMER-REAL`, `SIRI-INTENTS`, and `USE-ACTION-PROBE` also support simulation. Simulation uses synthetic input and output and never invokes the owner's Shortcuts library. Real probe evidence contains only counts, digests, and assertion results. Missing or duplicate probe workflows fail closed without executing a shortcut.

F9c qualifies the implementation as well as the static metadata: packaging rejects an executable whose symbol table lacks a declared intent type. Its real runner launches the freshly built signed Helper at an explicit path in `intent-host` mode. This mode registers that bundle and uses the AppKit application event loop without opening setup or checking unrelated permissions. It prevents a metadata-only or stale temporary app with the same bundle identifier from supplying the execution host. The qualified Helper replaces the metadata-only installation at its stable application path. The host exits when its runner exits; the runner also terminates its own host and removes staging files. SIRI-INTENTS then invokes the shortcut again with that host stopped and requires a successful cold launch.

The host reports Shortcuts import and privacy dialogs and Clock confirmation sheets, including their exact displayed text and buttons, as single lines in `~/F9-DIALOG.txt`. It does not approve them. Startup and real shortcut execution allow up to five minutes for the orchestrator to resolve a dialog. The corrected signed Echo workflow is included in the repository for replacement import.

The Helper's `clock-timers` qualification adapter was removed: only retired Go runs called it and no operation reaches it. Timers go through the AB Clock pack.

The Screen Time action probe now exercises the bridge's existing read-only usage capture rather than requiring a nonexistent shortcut action. See `packaging/shortcuts/screen-time-probe.md` for its backend, independent comparison and limits.

Clock alarm readback uses the native hour and minute properties instead of coercing date components into a date. The pack formats times consistently and treats repeat weekdays as an unordered set. Workflow text fields, including Stop and Output and alarm names, use text-token strings; property references inside those strings resolve to raw attachments. Clock intent actions disable Show When Run so successful changes return without waiting for a Done sheet.

X2 traced the test Mac failure to the App Intents connection handshake. Signed metadata, implementation symbols, indexing and action discovery were present. linkd selected another running Helper in `session` mode and sent it `lnec/lnei`; that process registered with App Intents but its Foundation run loop never dispatched the connection Apple Event, producing `errAETimeout` (-1712). Previously exited targets also produced `procNotFound` (-600). Sessions now run AppKit's application event loop, terminating when their gateway session ends, so they can answer connection events when selected. Bundle replacement also unregisters and removes the predecessor before registering the signed host. Real qualification passed with the AppKit host and a separate cold SwiftUI launch, returning 60 bytes with all assertions passing. No other active Helper was terminated to obtain the pass.
