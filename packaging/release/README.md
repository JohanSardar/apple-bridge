# Local release pipeline

The release app supports Apple Silicon and macOS 26 or later. Nothing in these scripts publishes a GitHub release, uploads a public download, adds testers, or submits an App Store review.

## Mac release

Run `packaging/release/check.sh` for the focused appcast, numeric version, Ed25519 and uninstall tests. `go run ./cmd/bridge-lab run lab/scenarios/packaging/R1-RELEASE-SIM.yaml --level sim` records the same checks as lab evidence.

Provide these environment variables through your private shell configuration:

| Variable | Value |
| --- | --- |
| `DEVELOPER_ID_APPLICATION` | SHA-1 identity from `security find-identity -v -p codesigning`, specifically Developer ID Application |
| `ASC_KEY_PATH` | Absolute path to the App Store Connect `.p8` key |
| `ASC_KEY_ID` | The ASC key ID |
| `ASC_ISSUER_ID` | The ASC issuer ID |
| `SPARKLE_KEY_FILE` | Absolute path to a mode-600 file containing a base64-encoded 32-byte Ed25519 private seed |

Keep private keys outside git. A Sparkle seed can be generated without a keychain prompt with `umask 077; openssl rand -base64 32 > /private/path/update.seed`. Preserve this seed across releases: previous apps trust its public key. Do not use Sparkle's `generate_keys` here, because it writes the keychain. The Developer ID identity must be usable without a password prompt. `Identity.swift` can prepare a CSR using a nonexportable RSA key stored only in the Pro keychain and import Apple's public certificate. It explicitly disables Security framework interaction. Compile it with `ReleaseCore.swift`, run `identity csr /private/path/request.csr`, use `asc certificates create --certificate-type DEVELOPER_ID_APPLICATION --csr /private/path/request.csr`, decode the returned public certificate to DER, then run `identity import-certificate /private/path/certificate.der`. Stop if the keychain refuses access; do not unlock it or change its ACLs with a password during unattended work. Apple Development and iPhone Distribution identities cannot sign a downloadable Mac release.

```sh
python3 packaging/release/release.py \
  --version 0.1.1 --build 2 \
  --feed-url https://downloads.example.com/applebridge/appcast.xml \
  --download-url https://downloads.example.com/applebridge/AppleBridge-update.zip \
  --output dist/local-0.1.1
```

Use real HTTPS endpoints only when preparing a distribution. This command does not publish to them. Choose a new output directory for each attempt. `--prepare-unsigned` compiles and assembles only, skips Apple credentials and notarization, and creates no distributable DMG. This is suitable for local compilation checks with a disposable seed; do not distribute that app.

Sparkle 2.10.1 is downloaded with a pinned SHA-256. The release launcher links the embedded framework; Sparkle performs download verification, update installation and relaunch. The launcher checks for updates automatically, exposes Check for Updates, and restarts an enabled background gateway after an update relaunch. Users enable Launch at Login explicitly through SMAppService. Setup opens the existing permission and background-service window.

The pipeline signs nested Mach-O binaries and Sparkle bundles from the inside out, with secure timestamps and hardened runtime. It preserves Sparkle's internal entitlements, signs the helper with the existing automation/device grants, and signs the outer app last. It notarizes the app ZIP using `notarytool --key --key-id --issuer`, staples and assesses the app, then creates the final update ZIP from the stapled app. Both the ZIP enclosure and appcast XML are Ed25519 signed. A CryptoKit verification independently checks the ZIP signature and byte length before the pipeline proceeds. The DMG contains the app and an Applications symlink for drag installation. It is signed, notarized, stapled and checked using:

```sh
xcrun stapler validate dist/local-0.1.1/AppleBridge.dmg
spctl -a -vv --type open --context context:primary-signature dist/local-0.1.1/AppleBridge.dmg
APPLE_BRIDGE_RELEASE_DMG="$PWD/dist/local-0.1.1/AppleBridge.dmg" \
  go run ./cmd/bridge-lab run lab/scenarios/packaging/R1-DMG-REAL.yaml --level real
```

`evidence.json` contains only pass/fail and the DMG digest. Notary rejection stops the build; use `xcrun notarytool log` with the same path-based credentials to inspect Apple's submission separately. The development packager remains available for lab scenarios.

## Previous-version and uninstall acceptance

After obtaining a Developer ID identity, build two releases with consecutive build numbers and the same Sparkle seed. Copy the previous app into a user-owned temporary directory. Configure its HTTPS feed URL to a private test feed before signing that previous build. Serve the new signed feed and archive at that private endpoint. Open that copy, select Check for Updates and Install and Relaunch, and verify its CFBundleVersion increases, its signature still passes `codesign --verify --deep --strict`, and enabled sync resumes. Do not override HOME, install over an existing personal app, or publish the test feed. This interactive Sparkle installation has not yet been proved by this ticket; archive signature tests alone do not prove it.

Uninstall in the release launcher requires explicit confirmation. It unregisters the gateway and main-app login services, stops its setup helper, deletes the dedicated credential keychain if present, resets the app's TCC permissions, removes only the application's standard data/cache/preferences/saved-state paths, and moves the entire app (including bundled helper and Sparkle) to the Trash. Cleanup refuses redirected ancestor directories and stops if unregistering, credential removal or permission reset fails. The simulated cleanup uses only temporary tagged files and proves unrelated data is preserved. Real keychain deletion, TCC reset and SMAppService unregister are never executed by the unit tests.

macOS may retain protected Full Disk Access or Accessibility list entries after `tccutil reset`. The uninstall confirmation tells the user to remove those entries in System Settings. No supported public API removes all protected permission-list entries without user involvement. A fresh-user uninstall remains a real acceptance check and must not be inferred from the unit tests.

## iOS TestFlight path

Provide `APPLE_DEVELOPER_TEAM_ID`, `IOS_DISTRIBUTION_IDENTITY`, and the same three ASC credential variables. Create a private JSON mapping each bundle ID below to its App Store distribution `.mobileprovision` path:

- `com.johansardar.applebridge.ios`
- `com.johansardar.applebridge.ios.alarms`
- `com.johansardar.applebridge.ios.widgets`
- `com.johansardar.applebridge.ios.screentimemonitor`
- `com.johansardar.applebridge.ios.screentimereport`

The main app and Screen Time extensions need Apple's distribution approval for their restricted entitlements. An iOS development profile or APNs key does not supply this approval. The ASC app record must also exist for the main bundle ID.

```sh
python3 packaging/release/testflight.py \
  --profiles /private/path/app-store-profiles.json \
  --version 0.1.1 --build 2 --output dist/testflight-2
```

This validates profile types and bundle/team matches, archives Release with production APNs and consistent app/extension versions, and exports an IPA using manual signing. Add `--upload-internal` to upload that IPA to App Store Connect with `altool` and the path-based API key. Export uses `testFlightInternalTestingOnly=true`; no external tester groups, beta review or public distribution are configured. Upload is opt-in, and no upload was performed for this ticket.

## Observed local blockers

- The Pro's installed identities and the ASC certificate list contain no Developer ID Application identity. The explicitly authorized, noninteractive key-generation attempt returns errSecAuthFailed (-25293) in both CLI and GUI sessions, so no CSR or certificate could be created without user authentication. Notarized DMG, Gatekeeper and previous-version installation acceptance remain unproved.
- ASC lists no app for the iOS bundle ID and no Apple Bridge App Store profiles. The existing iOS profiles are development profiles; no TestFlight archive/export/upload is claimed.
- Fresh-user setup/uninstall and removal of protected permission-list entries require user/OS access unavailable under the no-password/no-prompt rule.
