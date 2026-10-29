# iPhone companion

Requires Xcode 26.4+, XcodeGen (`brew install xcodegen`), an Apple Development signing identity and a matching development profile at `~/.config/apple-bridge/ios/dev.mobileprovision`.

Run `packaging/dev/build-ios.sh`, then `packaging/dev/install-ios.sh`. The installer copies only our IPA to the Air and installs over USB with pymobiledevice3. Build products and generated Xcode projects are ignored. Override the profile with `APPLE_BRIDGE_IOS_PROFILE` or output directory with `APPLE_BRIDGE_IOS_OUTPUT`.

The app uses manual signing. Extension targets (widgets, App Intents and AlarmKit features) must use their own matching provisioning profiles before embedding. The current profile has no App Group identifiers; shared containers must be provisioned before use. App Intents and AlarmKit can also be implemented in the main app target.

Capability work is isolated under `ios/Capabilities/<Name>/`. Each folder owns its descriptor and future views, permission adapter and handlers. Register it once in the append-only `ios/App/Capabilities.swift` list. XcodeGen includes the entire directory automatically. Keep registry entries from both sides when resolving merges.

APP-SHELL-SIM: build for iphonesimulator with signing disabled, install, launch, open a capability and check its switch/permission/activity fields, then open Diagnostics and verify the registered capability count. APP-SHELL-REAL: build, install via Air, and launch our bundle.
