# Signed shortcut builds (G0-6)

`go run ./tools/shortcuts build` compiles every `packaging/shortcuts/*.json` source into `.build/shortcuts/`, then signs each workflow using the system `/usr/bin/shortcuts sign`. Use `-source path.json` for one source, `-out directory` for another build directory, and `-unsigned` for portable compilation without signing. Signing defaults to `-mode auto`: try `anyone`, then `people-who-know-me`. Explicit modes disable fallback. The JSON stdout and `manifest.json` record the actual mode and unsigned SHA-256 for each file. A failed signer exits nonzero and never publishes its partial signed file.

`go run ./tools/shortcuts open ".build/shortcuts/[ab-test noop] Apple Bridge Signing Test.shortcut"` opens the signed file with macOS `open` for the user's import review. Build never opens or imports a workflow. CORE-SHORTCUT-SIGN never opens, imports or runs it.

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

`internal/shortcut` invokes the CLI directly without shell expansion. Inventory uses `list --show-identifiers`, optionally `--folders` or `--folder-name`; malformed or duplicate identifiers reject the entire result. CLI names cannot establish input capabilities, so none are inferred. The native integration below registers capture and text dispatch.

Runs accept input bytes (text, URL text or file content) and return output bytes, process exit status and timeout status. Private temporary directories are removed after every result. Input and output are capped at 16 MiB. The default timeout is 30 seconds, bounded to five minutes; cancellation kills the CLI process group and reaps the CLI process. It does not promise to cancel downstream work that Shortcuts may already have handed to an app. Process success proves only runner completion, never arbitrary shortcut business effects. The adapter intentionally discards process stdout/stderr to avoid retaining personal diagnostics.

`bridge shortcuts install-pack [-pack directory]` validates all three signed AEA files before opening them for user review. It never clicks Add Shortcut. The required files are `AB Echo.shortcut`, `AB Clock.shortcut` and `AB Focus.shortcut`; only the `AB Echo` source is supplied in this partial change. Missing Clock/Focus files cause the installer to reject before opening anything. Coverage publishes the bundled protocol version; imported pack version qualification remains pending.

SC-CAPTURE sim tests inventory adapter changes and native capture, not replication. SC-RUN sim tests byte input/output, binary output, exit status, timeout, invalid shortcut names and input size. Complete pack sources, Clock, Focus, actions discovery and file transforms remain outstanding. No real shortcut has been run. The leader can import the signed Echo artifact for subsequent real qualification; that import alone does not complete T26.

## Native capture and text dispatch continuation

The Helper registers `ShortcutsSource`, polls inventory every 30 seconds, and emits complete replacement checkpoints only after all folder inventories succeed. Failed scans never assert absence. Shortcut input capability remains null because the CLI cannot establish it. `shortcuts:read` and `shortcuts:write` join the grant registry. Coverage carries `pack_version: "1"` through the gateway; this is the bundled protocol version, not proof of an imported pack version.

`shortcuts.run` is registered as a command and follows the normal admission, approval, ticket and dispatch guard path. Its current input is shortcut name, UTF-8 input (16 KiB maximum) and timeout_ms (default 30 seconds, maximum five minutes). Native dispatch uses private temporary files and direct process arguments, starts the CLI in a dedicated process group, kills the whole group at its deadline or task cancellation, and reaps the CLI, and returns exit_status and output_base64 (32 KiB maximum output, keeping command evidence within IPC frame limits). A nonzero exit remains indeterminate because downstream effects may have occurred. Successful evidence establishes runner completion only. Its observed command has no capture commit sequence or observed record IDs; the ledger permits this result-only outcome specifically for shortcuts.run. File blob grants and blob output integration remain unimplemented, so T26 is still partial.

SC-CAPTURE and SC-RUN additionally compile and exercise the native actor with the fake CLI. SC-RUN also submits through the public REST gateway against SimApple. These checks do not prove capture replication or real pack behavior. The idle qualification remains pending. No Clock or Focus identifiers have been inferred, and no placeholder pack workflows claim working operations.

## Review safety fixes

The Go adapter bounds stdout as it streams and monitors CLI output-file growth every 5 ms, killing the process group when either exceeds 16 MiB. Native output monitoring retains its 32 KiB run-result bound. These bounds cover CLI descendants in the group; effects already handed to independent applications remain outside runner cancellation.

The gateway waits for the configured shortcuts execution timeout (default 30 seconds, maximum five minutes) plus five seconds for completion transport. Admission expiry still fences dispatch. Resolve accepts retained successful runner-completion evidence tied to the prepared request without requiring an Apple object capture.

SC-RUN uses only synthetic executables: a forking timeout fixture checks descendant termination in Go and Swift; continuous file/stdout writers prove termination before their execution deadline; a native 16-second success proves retained Resolve evidence, and a separate 16-second SimApple success proves the public gateway waits beyond 15 seconds. Absolute times are not scaled.

## App Intents discovery (T29 slice)

`shortcuts.actions` is a read-only query under `shortcuts:read`, taking an empty argument object. It reads installed application metadata under /Applications, /System/Applications and the current user's Applications directory, including extension bundles. It never launches applications or invokes an action. Results contain bundle identifier, action identifier and title, plus completeness and `schema_guaranteed: false`. Metadata is an undocumented, version-dependent inventory; missing metadata does not prove an app has no actions.

Traversal skips symlinks and hidden entries, caps depth at 12, visits at 50,000, time at ten seconds, each metadata file at 4 MiB, total metadata at 32 MiB, rows at 2,000 and serialized output conservatively below 900 KB. Malformed metadata and traversal failures mark the result incomplete. Hidden and assistant-only actions are excluded. SC-ACTIONS sim generates a synthetic bundle, tests malformed metadata and symlink exclusion, and exercises the public query against SimApple. Real reads only installed metadata and retains pass/fail evidence without action names or app identifiers.

`files.transform` command integration and FILES-TRANSFORM remain pending in this partial T29 slice; no transform capability is advertised.

Installed pack qualification uses a unique `AB Pack v1` inventory marker and exactly one each of AB Echo, AB Clock and AB Focus from bounded `shortcuts list --show-identifiers`. Missing, duplicate or unavailable inventory omits `pack_version`; the bundled version alone never establishes installation. This marker establishes inventory presence, not workflow integrity. Import the marker alongside the signed pack.

`shortcuts.run` accepts mutually exclusive UTF-8 `input` or inline file/blob bytes in `input_base64`, capped at 16 KiB decoded. Output remains base64 encoded, capped at 32 KiB, with `output_bytes` and SHA-256 evidence. No caller filesystem path is accepted. Large uploaded blobs remain unsupported.
