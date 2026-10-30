# Real sensor qualification

Build and install the app after changing Swift code. Reuse that installation for subsequent runs. Set `APPLE_BRIDGE_REAL_DEVICE` to the physical device identifier from `xcrun devicectl list devices`, then run:

```
go run ./cmd/bridge-lab run lab/scenarios/iphone/APP-HEALTH-REAL.yaml --level real --json
```

The eight sensor scenarios launch the app with `--real-qualification <scenario> <fresh UUID>`. Each run holds `lockf -k -t 3600 /tmp/ab-iphone.lock` until the result is copied or the deadline expires. Keep installations under the same lock when sharing the phone. The harness never builds or installs automatically.

The app writes a protected, atomic result in its Documents/qualification directory. Only the current UUID's final result qualifies. The host exports checks, counts and SHA-256 digests. Sample contents, media, location coordinates, Wi-Fi names and device names are excluded. Capture bytes stay in memory and are discarded; no Photos or recordings are created. Health writes use the existing tagged write/read/delete proof. The alarm uses a unique tagged identity, is scheduled an hour ahead, and is canceled immediately after readback. Screen Time uses a separate tagged store and monitor, clears them on exit, and preserves the existing selection and Bridge policies. Location removes its own tagged region.

Permission instructions are printed before launching. Allow up to three minutes per authorization step. The capture run permits two successive Face ID and sensor permission steps. A prompt instruction does not prove that a dialog appeared or that anyone tapped it. Record taps only when observed. No calls are made. NFC is explicitly skipped because the scenario requires a physical tag.

A successful API call is not complete acceptance proof. Empty Health reads fail because HealthKit hides read-denial status. Missing Wi-Fi information fails its check. Screen Time callback/report observation, background location events, and capture's signed gateway upload remain explicit failed checks until observed. These scenarios must remain limited even when their local operations succeed.
