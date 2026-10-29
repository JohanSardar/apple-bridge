# iPhone companion

Requires Xcode 26.4+, XcodeGen (`brew install xcodegen`), an Apple Development signing identity and a matching development profile at `~/.config/apple-bridge/ios/dev.mobileprovision`.

Run `packaging/dev/build-ios.sh`, then `packaging/dev/install-ios.sh`. The installer copies only our IPA to the Air and installs over USB with pymobiledevice3. Build products and generated Xcode projects are ignored. Override the profile with `APPLE_BRIDGE_IOS_PROFILE` or output directory with `APPLE_BRIDGE_IOS_OUTPUT`.

The app uses manual signing. Extension targets (widgets, App Intents and AlarmKit features) must use their own matching provisioning profiles before embedding. The current profile has no App Group identifiers; shared containers must be provisioned before use. App Intents and AlarmKit can also be implemented in the main app target.

Capability work is isolated under `ios/Capabilities/<Name>/`. Each folder owns its descriptor and future views, permission adapter and handlers. Register it once in the append-only `ios/App/Capabilities.swift` list. XcodeGen includes the entire directory automatically. Keep registry entries from both sides when resolving merges.

APP-SHELL-SIM: build for iphonesimulator with signing disabled, install, launch, open a capability and check its switch/permission/activity fields, then open Diagnostics and verify the registered capability count. APP-SHELL-REAL: build, install via Air, and launch our bundle.

Pairing: on the Mac run `bridge pair --iphone --via-remote`, then scan the QR from the app’s Pairing screen (or paste its `ab1-` code). The app stores Ed25519 signing and X25519 encryption keys in a device-only Keychain item accessible after first unlock. The phone-only one-time proof binds both public keys to the code. Replica enrollment rejects phone codes. The phone receives no gateway API grants. `iphone.devices` lists device IDs for `bridge revoke <id>`.

Remote pairing validates the relay’s normal HTTPS certificate. Direct test HTTPS pins the certificate fingerprint in the QR. The replication mTLS port is not an iPhone endpoint. For isolated testing, `APPLE_BRIDGE_SECRET_STORE=file go run ./lab/iphonegateway -home /tmp/ab-test-iphone -address https://<reachable-host>:17781` writes a private code file and serves an iOS-compatible TLS certificate. `--pair-code <code>` is an app launch seam for unattended lab pairing; do not put the code in evidence.

APP-PAIR-SIM runs with `APPLE_BRIDGE_SECRET_STORE=file go run ./cmd/bridge-lab run lab/scenarios/iphone/APP-PAIR-SIM.yaml --level sim`. Physical QR/local-network permission prompts require Johan’s Allow tap.
