# Signed shortcut builds (G0-6)

`go run ./tools/shortcuts build` compiles every `packaging/shortcuts/*.json` source into `.build/shortcuts/`, then signs each workflow using the system `/usr/bin/shortcuts sign`. Use `-source path.json` for one source, `-out directory` for another build directory, and `-unsigned` for portable compilation without signing. Signing defaults to `-mode auto`: try `anyone`, then `people-who-know-me`. Explicit modes disable fallback. The JSON stdout and `manifest.json` record the actual mode and unsigned SHA-256 for each file. A failed signer exits nonzero and never publishes its partial signed file.

`go run ./tools/shortcuts open .build/shortcuts/noop.shortcut` opens the signed file with macOS `open` for the user's import review. Build never opens or imports a workflow. CORE-SHORTCUT-SIGN never opens, imports or runs it.

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

This intentionally small schema carries native parameters rather than maintaining a second action catalog. Later tickets can supply verified native identifiers/parameters for Run Shortcut, current weather/forecast, Clock alarms/timers, Focus, battery, Text, Dictionary, Get Dictionary Value, If/Otherwise, Send Message and Get Contents of URL (HTTP). Dictionary item serialization, token strings and condition modes are passed as nested native objects. This ticket proves compilation/signing of Nothing only; it does not claim those later actions' platform availability, grants or business effects. Consult capability catalog S01–S10 and the iMCP port map before exposing execution.

## Verification boundary

CORE-SHORTCUT-SIGN at sim compiles twice and compares bytes/digests, then independently parses the WFWorkflow plist with macOS `plutil`. At real it additionally signs the no-op source, checks AEA1 framing, and re-signs that signed file with the system CLI to prove Apple accepts the archive. Evidence records pass/fail, digests and signing mode only. No personal library content is read. There are no new runtime Sources or grants. Signed files are build artifacts, excluded under `.build/`.

## Runner adapter (T26, partial)

`internal/shortcut` invokes the CLI directly without shell expansion. Inventory uses `list --show-identifiers`, optionally `--folders` or `--folder-name`; malformed or duplicate identifiers reject the entire result. CLI names cannot establish input capabilities, so none are inferred. This adapter is not yet registered as a captured Source or public command.

Runs accept input bytes (text, URL text or file content) and return output bytes, process exit status and timeout status. Private temporary directories are removed after every result. Input and output are capped at 16 MiB. The default timeout is 30 seconds, bounded to five minutes; cancellation kills and reaps the CLI process. It does not promise to cancel downstream work that Shortcuts may already have handed to an app. Process success proves only runner completion, never arbitrary shortcut business effects. The adapter intentionally discards process stdout/stderr to avoid retaining personal diagnostics.

`bridge shortcuts install-pack [-pack directory]` validates all three signed AEA files before opening them for user review. It never clicks Add Shortcut. The required files are `ab-echo.shortcut`, `ab-clock.shortcut` and `ab-focus.shortcut`; only the `AB Echo` source is supplied in this partial change. Missing Clock/Focus files cause the installer to reject before opening anything. `PackVersion` is reserved for later coverage reporting and is not yet exposed in `sources.status`.

SC-CAPTURE sim tests inventory adapter changes, not replication. SC-RUN sim tests byte input/output, binary output, exit status, timeout, invalid shortcut names and input size. Native capture/command wiring, complete pack sources, Clock, Focus, actions discovery and file transforms remain outstanding. No real shortcut has been run. The leader can import the signed Echo artifact for subsequent real qualification; that import alone does not complete T26.
