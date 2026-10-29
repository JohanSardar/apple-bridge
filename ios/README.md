# iPhone companion

Requires Xcode 26.4+, XcodeGen (`brew install xcodegen`), an Apple Development signing identity and a matching development profile at `~/.config/apple-bridge/ios/dev.mobileprovision`.

Run `packaging/dev/build-ios.sh`, then `packaging/dev/install-ios.sh`. The installer copies only our IPA to the Air and installs over USB with pymobiledevice3. Build products and generated Xcode projects are ignored. Override the profile with `APPLE_BRIDGE_IOS_PROFILE` or output directory with `APPLE_BRIDGE_IOS_OUTPUT`.

The app uses manual signing. Extension targets (widgets, App Intents and AlarmKit features) must use their own matching provisioning profiles before embedding. The current profile has no App Group identifiers; shared containers must be provisioned before use. App Intents and AlarmKit can also be implemented in the main app target.
