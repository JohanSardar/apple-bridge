# Local release pipeline

The Mac release supports Apple Silicon and macOS 26 or later. The scripts do not publish a public download, add testers, or submit an App Store review.

## Dedicated signing keychain

Configure `asc` with its existing file-based App Store Connect API credentials, then run:

```sh
python3 packaging/release/signing.py
python3 packaging/release/signing.py --certificate-type DISTRIBUTION
```

The first command provisions Developer ID Application for the Mac; the second provisions Apple Distribution for iOS. Both use `~/.config/apple-bridge-release/ab-release.keychain-db`, never the login keychain. The directory is mode 700; its randomly generated `keychain.password`, OpenSSL private keys, CSRs, certificates, and API responses are mode 600. Keep that entire directory outside git and preserve it between releases. ASC credential resolution bypasses the login keychain.

Setup unlocks this dedicated keychain, disables its automatic locking, imports the OpenSSL private key and Apple's certificate, sets the `apple-tool:,apple:,codesign:` partition list, and preserves the existing search list while adding the release keychain. It saves the identity fingerprints and keychain path in the private `signing.json`. Rerunning reuses the existing password, key, CSR, and issued certificate. It never revokes a certificate. A missing password for an existing keychain stops setup.

Export these values from your private configuration:

| Variable | Value |
| --- | --- |
| `RELEASE_KEYCHAIN` | Absolute path to `ab-release.keychain-db` |
| `DEVELOPER_ID_APPLICATION` | Developer ID SHA-1 identity from private `signing.json` |
| `IOS_DISTRIBUTION_IDENTITY` | Apple Distribution SHA-1 identity from private `signing.json` |
| `ASC_KEY_PATH` | Absolute path to the existing ASC `.p8` key |
| `ASC_KEY_ID` | Its key ID |
| `ASC_ISSUER_ID` | Its issuer ID |
| `SPARKLE_KEY_FILE` | Absolute path to a mode-600 file containing a base64 32-byte Ed25519 seed |

Generate a new update seed only for the first release, using `umask 077; openssl rand -base64 32 > /private/path/update.seed`. Preserve it: previous releases trust its public key. No command needs Johan's login password. The old login-keychain `Identity.swift` path has been removed.

## Mac release

```sh
python3 packaging/release/release.py \
  --version 0.1.2 --build 3 \
  --feed-url https://downloads.example.com/applebridge/appcast.xml \
  --download-url https://downloads.example.com/applebridge/AppleBridge-update.zip \
  --output dist/local-0.1.2
```

Use real HTTPS endpoints when distributing. Choose a new output directory. `--prepare-unsigned` compiles and assembles an app for local checks and produces no distributable DMG.

Sparkle 2.10.1 is pinned by SHA-256. The launcher links its embedded framework and offers automatic updates, Check for Updates, explicit Launch at Login through SMAppService, and permission setup. The gateway restarts after an update if it was enabled.

All nested Mach-O files, Sparkle bundles, the outer app, and the DMG are signed with an explicit `codesign --keychain` argument. Identity lookup is restricted to that keychain and requires Developer ID Application. The pipeline notarizes the app ZIP with path-based ASC credentials, staples and assesses the app, signs both the final ZIP enclosure and appcast with Ed25519, and independently verifies the archive signature and length. It creates a drag-to-Applications DMG, notarizes and staples it, then requires both checks:

```sh
xcrun stapler validate dist/local-0.1.2/AppleBridge.dmg
spctl -a -vv -t open --context context:primary-signature dist/local-0.1.2/AppleBridge.dmg
```

`evidence.json` contains pass/fail and the DMG digest. Rejection stops the build. Inspect a rejected submission with `notarytool log` and the same ASC key arguments.

## Update and uninstall checks

Run `packaging/release/check.sh` or the `R1-RELEASE-SIM` scenario for appcast, numeric version, signature/tamper, explicit keychain selection, signing-password protection, export-tool selection, and uninstall failure/cleanup tests.

To exercise Sparkle's actual network, signature verification, extraction and installer against a writable temporary copy:

```sh
python3 packaging/release/update_check.py --app '/path/to/Apple Bridge.app'
```

This check requires the private `IOS_DISTRIBUTION_IDENTITY` and `RELEASE_KEYCHAIN` variables. It copies build 2 and build 3, signs both with the dedicated Apple Distribution identity and the same disposable update seed, and serves a signed fixture feed on loopback HTTP. Only these private fixtures permit HTTP. The custom user driver accepts the update; Sparkle installs build 3, and the check verifies its version and full code signature. It relaunches only the disposable driver, which exits immediately. It does not launch the product, enable login items, or replace a personal installation. This proves the real installer separately from Developer ID/Gatekeeper qualification.

The install/uninstall check uses the same cleanup implementation as the launcher:

```sh
swiftc -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk \
  -swift-version 6 -warnings-as-errors -parse-as-library \
  packaging/release/ReleaseCore.swift packaging/release/UninstallCheck.swift \
  -o .build/release-checks/uninstall-check
.build/release-checks/uninstall-check '/path/to/Apple Bridge.app'
```

It copies the app and bundled helper into a temporary HOME under the user's cache, installs actual credentials in the app's own dedicated keychain, creates state/cache/preferences fixtures, runs `tccutil reset All dev.applebridge.r1test.uninstall`, deletes that keychain, and uninstalls. It checks that no files remain under the temporary HOME. LaunchServices must see the fixture before TCC reset; `/var/folders` fixtures are ignored. No desktop app launches with an overridden HOME, no actual user grants are reset, and no login service is registered by the check.

The product uninstall explicitly confirms removal, unregisters the gateway and main-app login services, stops the setup helper, deletes the app credential keychain, resets `dev.applebridge.helper` TCC permissions, clears updater preferences and standard app data paths, and removes the app bundle including its helper. Redirected ancestors are refused; unregister, credential and TCC failures stop cleanup. `R1-DMG-REAL` runs locally on the Pro. It checks the stapled DMG, requires Gatekeeper to report `accepted` and `source=Notarized Developer ID`, mounts it read-only, verifies the bundled app with `codesign --verify --deep --strict`, validates its staple, checks the Applications link, and detaches the volume. It does not launch the product or change permissions.

## TestFlight

All five bundle IDs already exist. App Store profiles have now been created for the main app, alarms, widgets, Screen Time monitor and Screen Time report, using the new dedicated Apple Distribution certificate. The profiles include production APNs and the required Family Controls distribution entitlements. Their paths are saved privately in `~/.config/apple-bridge-release/profiles-release.json`.

```sh
export APPLE_DEVELOPER_TEAM_ID=AX3C2B78YD
python3 packaging/release/testflight.py \
  --profiles "$HOME/.config/apple-bridge-release/profiles-release.json" \
  --version 0.1.2 --build 3 --output dist/testflight-3 \
  --upload-internal
```

The script checks profile types and bundle/team matches, archives with an explicit signing keychain, exports an internal-only TestFlight IPA, and uploads with `altool` using the ASC API key. Xcode runs with Apple tools first in PATH because Homebrew rsync rejects Apple's extended-attribute arguments during IPA export. No external tester groups or review submission are configured.

The App Store Connect iOS app record is `Apple Bridge`, bundle `com.johansardar.applebridge.ios`, SKU `APPLEBRIDGE-IOS`, app ID `6829322065`. The upload stays internal-only through `testFlightInternalTestingOnly=true`; no public release, external tester group, or App Store review is created.

The app icon uses the existing web icon in an opaque 1024-pixel iOS asset catalog. iPad orientation metadata supports multitasking, the obsolete NFC `NDEF` entitlement is excluded, and the Device Activity report uses ExtensionKit and the app's `Extensions` directory. These checks run in `R1-RELEASE-SIM` alongside the release tests.

Confirm Apple's independent processing result after upload:

```sh
ASC_BYPASS_KEYCHAIN=1 asc builds list --app 6829322065 \
  --version 0.1.2 --build-number 3 --processing-state all --output json
```

## Local qualification

The supplied Developer ID Application certificate is installed with its private key in the dedicated release keychain. Unlock only that keychain using the saved private password before running the Mac pipeline. Keep all keys, passwords, profiles, and ASC credentials outside git.

The local update feed uses `example.invalid` intentionally. Replace it with a hosted HTTPS feed and enclosure URL before distributing a public download. No public download or update feed is published by these checks.
