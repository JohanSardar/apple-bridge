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

Generate a new update seed only for the first release, using `umask 077; openssl rand -base64 32 > /private/path/update.seed`. Preserve it: previous releases trust its public key. No command needs the owner’s login password. The old login-keychain `Identity.swift` path has been removed.

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

## npm package

`packaging/npm` holds a Mac-only package that installs the `bridge` command. The package name is on one line, in `packaging/npm/package.json`. Nothing has been published, and user documentation does not mention npm until it is.

The tarball carries one signed `Apple Bridge.app` from `packaging/build-app.sh`: the Helper, `bridge`, `ab-session` and the gateway LaunchAgent, with no Sparkle and no frameworks. npm tarballs cannot hold symlinks or empty folders, and a code signature only seals file contents, so this flat app survives install byte for byte. `build.py` refuses an app with a symlink, an empty folder or a framework. `bin/bridge` is a small shell launcher that runs the `bridge` inside the app, so that binary finds its app and its Keychain store the same way it does in the DMG. There is no install script.

Signing happens locally because the Developer ID key stays in the release keychain, so a publish is run by hand and has no npm provenance. Users can check a package with `codesign -dv` on the installed app, which names the team.

To publish, on the Mac that holds the release keychain, from a clean checkout of the release commit, with `name` set to the package name and `version` to the workspace version:

```sh
security unlock-keychain -p "$(cat ~/.config/apple-bridge-release/keychain.password)" "$RELEASE_KEYCHAIN"
identity=$(security find-identity -v -p codesigning "$RELEASE_KEYCHAIN" | awk '/Developer ID Application/{print $2; exit}')
out=dist/npm-$version
APPLE_BRIDGE_SIGNING_GUI=1 packaging/build-app.sh --identity "$identity" "$out/Apple Bridge.app"
ditto -c -k --keepParent "$out/Apple Bridge.app" "$out/notary.zip"
xcrun notarytool submit "$out/notary.zip" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait --timeout 20m
xcrun stapler staple "$out/Apple Bridge.app"
python3 packaging/npm/build.py --app "$out/Apple Bridge.app" --identity "$identity" --output "$out/package"
python3 packaging/npm/check.py --tarball "$out/package/$name-$version.tgz" --app "$out/Apple Bridge.app" --identity "$identity" --require-notarized
security lock-keychain "$RELEASE_KEYCHAIN"
npm login
npm publish "$out/package/$name-$version.tgz" --access public
```

`build.py` refuses an app that is not Developer ID signed, stapled and accepted by Gatekeeper. With `--proof` it accepts any signature, and keeps `"private": true` so npm refuses to publish the result. `check.py` installs into throwaway folders and runs `bridge` with a throwaway HOME and data folder. A stand-in replaces `open`, so it never launches the app or touches a permission.

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

The product uninstall explicitly confirms removal, unregisters the gateway and main-app login services, stops the setup helper, deletes the app credential keychain, resets `dev.applebridge.helper` TCC permissions, clears updater preferences and standard app data paths, and removes the app bundle including its helper. Redirected ancestors are refused; unregister, credential and TCC failures stop cleanup. `R1-DMG-REAL` runs locally on the build Mac. It checks the stapled DMG, requires Gatekeeper to report `accepted` and `source=Notarized Developer ID`, mounts it read-only, verifies the bundled app with `codesign --verify --deep --strict`, checks the Applications link, and detaches the volume. It does not launch the product or change permissions.

## iPhone releases

The iPhone archive, export and internal TestFlight tooling now lives in [apple-bridge-iphone](https://github.com/JohanSardar/apple-bridge-iphone). Run those commands from that repository. The dedicated release keychain can hold both existing identities; no certificate or credential migration is needed.

## Local qualification

The supplied Developer ID Application certificate is installed with its private key in the dedicated release keychain. Unlock only that keychain using the saved private password before running the Mac pipeline. Keep all keys, passwords, profiles, and ASC credentials outside git.

The local update feed uses `example.invalid` intentionally. Replace it with a hosted HTTPS feed and enclosure URL before distributing a public download. No public download or update feed is published by these checks.

The Pro qualification produced `dist/r1c-dmg3/AppleBridge.dmg` and the internal-only IPA at `dist/testflight-r1c-4/export/AppleBridge.ipa`. The DMG's mounted app passes strict signature verification, the Applications link is present, and the real Sparkle build-2-to-build-3 update passes. Apple reported both the app ZIP and DMG notarization submissions as In Progress after 20-minute waits. `notarytool history` on 2026-11-05 lists the app ZIP and DMG submissions of 2026-10-31 and 2026-11-04 as Accepted. Staple and Gatekeeper qualification of those DMGs has not been rerun.

TestFlight version 0.1.2 build 4 is independently confirmed as VALID and INTERNAL_ONLY with no beta groups and external testing NOT_APPLICABLE. ASC's internal build state is MISSING_EXPORT_COMPLIANCE; the encryption declaration remains to be completed before assigning internal testers.

If a notarization wait times out, preserve the signed artifact and query the existing submission with `notarytool history` and `notarytool info`, using the same path-based ASC key arguments. Once the DMG submission is Accepted, run `xcrun stapler staple`, the validation and Gatekeeper commands above, then rerun `R1-DMG-REAL` on the build Mac with `APPLE_BRIDGE_RELEASE_DMG` pointing at that DMG. Do not queue this check on the test Mac.
