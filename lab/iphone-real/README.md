# Real sensor qualification

Build and install the app after changing Swift code. Reuse that installation for subsequent runs. Set `APPLE_BRIDGE_REAL_DEVICE` to the physical device identifier from `xcrun devicectl list devices`, then run:

```
go run ./cmd/bridge-lab run lab/scenarios/iphone/APP-HEALTH-REAL.yaml --level real --json
```

The eight sensor scenarios launch the app with `--real-qualification <scenario> <fresh UUID>`. Each run holds `lockf -k -t 3600 /tmp/ab-iphone.lock` until the result is copied or the deadline expires. Keep installations under the same lock when sharing the phone. The harness never builds or installs automatically.

The app writes a protected, atomic result in its Documents/qualification directory. Only the current UUID's final result qualifies. The host exports checks, counts and SHA-256 digests. Sample contents, media, location coordinates, Wi-Fi names and device names are excluded. Capture bytes stay in memory and are discarded; no Photos or recordings are created. Health writes use the existing tagged write/read/delete proof. The alarm uses a unique tagged identity, is scheduled an hour ahead, and is canceled immediately after readback. Screen Time uses a separate tagged store and monitor, clears them on exit, and preserves the existing selection and Bridge policies. Location removes its own tagged region.

Permission instructions are printed before launching. Allow up to three minutes per authorization step. The capture run permits two successive Face ID and sensor permission steps. A prompt instruction does not prove that a dialog appeared or that anyone tapped it. Record taps only when observed. No calls are made. NFC is explicitly skipped because the scenario requires a physical tag.

A successful API call is not complete acceptance proof. Empty Health reads fail because HealthKit hides read-denial status. Missing Wi-Fi information fails its check. Screen Time callback/report observation remains explicitly unproved even when local authorization and scheduling succeed. Location qualifies the foreground fix and tagged geofence lifecycle; it does not claim background movement delivery. Capture qualifies Face ID, actual sensor bytes and discard; it does not claim a signed gateway round trip.

I1 prompt coordination: instructions append to `~/I1-NEEDS-JOHAN.txt`; set `AB_JOHAN_PROMPT_FILE` only when using a different coordination file. Fresh partial receipts identify each sensor prompt phase. Each phase gets three minutes for authorization, with thirty seconds for process/copy overhead. The phone lock fails immediately if busy so the operator can retry later. Sensor probes run only in Debug builds. Keep the phone connected and unlocked; `unavailable` in `devicectl list devices` blocks both installation and qualification.
