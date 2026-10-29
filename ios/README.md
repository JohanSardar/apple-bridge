# iPhone companion

Requires Xcode 26.4+, XcodeGen (`brew install xcodegen`), an Apple Development signing identity and a matching development profile at `~/.config/apple-bridge/ios/dev.mobileprovision`.

Run `packaging/dev/build-ios.sh`, then `packaging/dev/install-ios.sh`. The installer copies only our IPA to the Air and installs over USB with pymobiledevice3. Build products and generated Xcode projects are ignored. Override the profile with `APPLE_BRIDGE_IOS_PROFILE` or output directory with `APPLE_BRIDGE_IOS_OUTPUT`.

The app uses manual signing. Extension targets (widgets, App Intents and AlarmKit features) must use their own matching provisioning profiles before embedding. The current profile has no App Group identifiers; shared containers must be provisioned before use. App Intents and AlarmKit can also be implemented in the main app target.

Capability work is isolated under `ios/Capabilities/<Name>/`. Each folder owns its descriptor and future views, permission adapter and handlers. Register it once in the append-only `ios/App/Capabilities.swift` list. XcodeGen includes the entire directory automatically. Keep registry entries from both sides when resolving merges.

APP-SHELL-SIM: build for iphonesimulator with signing disabled, install, launch, open a capability and check its switch/permission/activity fields, then open Diagnostics and verify the registered capability count. APP-SHELL-REAL: build, install via Air, and launch our bundle.

Pairing: on the Mac run `bridge pair --iphone --via-remote`, then scan the QR from the app’s Pairing screen (or paste its `ab1-` code). The app stores Ed25519 signing and X25519 encryption keys in a device-only Keychain item accessible after first unlock. The phone-only one-time proof binds both public keys to the code. Replica enrollment rejects phone codes. The phone receives no gateway API grants. `iphone.devices` lists device IDs for `bridge revoke <id>`.

Remote pairing validates the relay’s normal HTTPS certificate. Direct test HTTPS pins the certificate fingerprint in the QR. The replication mTLS port is not an iPhone endpoint. For isolated testing, `APPLE_BRIDGE_SECRET_STORE=file go run ./lab/iphonegateway -home /tmp/ab-test-iphone -address https://<reachable-host>:17781` writes a private code file and serves an iOS-compatible TLS certificate. `--pair-code <code>` is an app launch seam for unattended lab pairing; do not put the code in evidence.

APP-PAIR-SIM runs with `APPLE_BRIDGE_SECRET_STORE=file go run ./cmd/bridge-lab run lab/scenarios/iphone/APP-PAIR-SIM.yaml --level sim`. Physical QR/local-network permission prompts require Johan’s Allow tap.

Delivery: the relay runs with `-apns-key <private-p8-path>` and `-apns-sandbox=true` for the development profile (`false` for production). Team, key ID and topic are fixed to the provisioned app. The gateway uses its existing private relay identity to authenticate `/iphone/push`; only signed ciphertext reaches the relay/APNs. Keys and APNs tokens must never be committed.

The app registers for silent pushes at launch; silent delivery does not need notification-alert permission. Pairing/local-network/camera prompts still need Johan. APNs background delivery is best effort, so the locked-phone under-five-second scenario must be measured on hardware and is not a latency guarantee. No production relay deployment is performed by these tickets.

`iphone.ping` takes `{"device_id":"iphone:…"}` inside the normal command envelope. Commands use the existing ledger, grants and owner approval. Admitted jobs survive restart; signed, encrypted jobs are dispatched on push or fetch. Expiry after possible dispatch is indeterminate. Device-only receipts prevent rerunning an ambiguous operation and retain signed results for upload after reconnecting.

Future capability folders supply `Capability.handlers`; the sole capability registration remains `App/Capabilities.swift`. Enabled, available capabilities contribute handlers automatically. Each gateway command owns an `iphone_<command>_descriptors.go` file and registers its argument validator with `RegisterIPhoneCommand`. Every argument object includes `device_id`. Handlers return JSON bytes; a handler that cannot prove whether an effect occurred must throw so the result is indeterminate. Keep permission checks inside the capability handler.

APP-CMD-SIM: `APPLE_BRIDGE_SECRET_STORE=file go run ./cmd/bridge-lab run lab/scenarios/iphone/APP-CMD-SIM.yaml --level sim`. It covers durable offline admission, encrypted relay delivery, signed results, replay/dedupe, expiry and read-agent denial. `go test ./internal/relay` verifies APNs JWT signing and background headers. The isolated gateway fixture writes its synthetic API token to `api-token` for Swift simulator round trips.

AlarmKit commands use `iphone.alarm.create/list/update/cancel` and `iphone.timer.start/cancel` through the existing push delivery path. Enable Alarms and timers in the app first. Arguments always include `device_id`; create/update take `fire_at` (Unix seconds), optional `title` and `snooze_seconds` (default 540). Update replaces the schedule for `id`; cancel takes `id`. Timer start takes `duration_seconds`. Optional UUID `id` on create/start allows callers to choose an identity. List reads the app's AlarmKit alarms, including their current system state. AlarmKit provides the system alarm sound through Silent and Focus, Stop, and a Snooze countdown. The first command requests AlarmKit authorization; permission must be allowed on the phone. APP-ALARM-SIM runs the production Swift command handler with a fake adapter. APP-ALARM-REAL remains limited until the Allow tap; final hardware proof schedules a tagged alarm a minute ahead, reads it back, then cancels it.

The embedded AlarmActivity widget renders AlarmKit countdowns on the Lock Screen and Dynamic Island. Signed builds additionally require `~/.config/apple-bridge/ios/alarms.mobileprovision` (override with `APPLE_BRIDGE_ALARM_PROFILE`) for `com.johansardar.applebridge.ios.alarms`. The build script installs both provisioning profiles and signs each target with its own profile.
