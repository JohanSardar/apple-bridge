#!/usr/bin/env python3
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import zipfile

from release import ROOT, require, run, version

BUNDLE = "com.johansardar.applebridge.ios"
PROFILE_SETTINGS = {
    BUNDLE: "APPLE_BRIDGE_APP_PROFILE",
    BUNDLE + ".alarms": "ALARM_ACTIVITY_PROFILE",
    BUNDLE + ".widgets": "AB_WIDGETS_PROFILE_NAME",
    BUNDLE + ".screentimemonitor": "AB_SCREENTIME_MONITOR_PROFILE_NAME",
    BUNDLE + ".screentimereport": "AB_SCREENTIME_REPORT_PROFILE_NAME",
}


def validate_profiles(paths, team):
    settings = {}
    decoded = {}
    for bundle, setting in PROFILE_SETTINGS.items():
        if bundle not in paths:
            raise ValueError(f"Provide an App Store distribution profile for {bundle}.")
        path = Path(paths[bundle]).expanduser()
        data = plistlib.loads(subprocess.check_output(["security", "cms", "-D", "-i", str(path)]))
        entitlements = data["Entitlements"]
        if data.get("ProvisionedDevices") or data.get("ProvisionsAllDevices") or entitlements.get("get-task-allow", False):
            raise ValueError(f"{bundle} needs an App Store distribution profile, not a development or ad hoc profile.")
        if data["TeamIdentifier"] != [team] or entitlements.get("application-identifier") != team + "." + bundle:
            raise ValueError(f"The distribution profile for {bundle} belongs to a different app or team.")
        if bundle == BUNDLE and entitlements.get("aps-environment") != "production":
            raise ValueError("The iOS distribution profile must enable production push notifications.")
        settings[setting] = data["Name"]
        decoded[bundle] = (path, data)
    return settings, decoded


def xcode_environment():
    environment = {k: v for k, v in os.environ.items() if k != "SDKROOT"}
    environment["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin:" + environment.get("PATH", "")
    return environment


def validate_internal_ipa(path):
    try:
        with zipfile.ZipFile(path) as archive:
            info = plistlib.loads(archive.read("Payload/AppleBridge.app/Info.plist"))
    except (zipfile.BadZipFile, KeyError, plistlib.InvalidFileException):
        raise ValueError("The IPA is missing valid app metadata. Export Apple Bridge again before uploading.") from None
    if info.get("CFBundleIdentifier") != BUNDLE or info.get("TFInternalTestingOnly") is not True:
        raise ValueError("Export Apple Bridge with internal-only TestFlight testing enabled before uploading.")


def main():
    parser = argparse.ArgumentParser(description="Archive/export iOS for TestFlight; upload requires --upload-internal. Never adds testers.")
    parser.add_argument("--profiles", required=True, help="Private JSON mapping bundle IDs to App Store profile paths")
    parser.add_argument("--output", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--build", required=True)
    parser.add_argument("--upload-internal", action="store_true")
    args = parser.parse_args()
    try:
        team = require("APPLE_DEVELOPER_TEAM_ID")
        marketing, build = version(args.version), version(args.build)
        destination = Path(args.output).resolve()
        if destination.exists():
            raise ValueError("Choose a new TestFlight output directory.")
        settings, profiles = validate_profiles(json.loads(Path(args.profiles).read_text()), team)
        identity = require("IOS_DISTRIBUTION_IDENTITY")
        keychain = Path(require("RELEASE_KEYCHAIN")).expanduser().resolve()
        if not keychain.is_file() or keychain.name == "login.keychain-db":
            raise ValueError("Use the dedicated release keychain prepared by signing.py.")
        for name in ["ASC_KEY_PATH", "ASC_KEY_ID", "ASC_ISSUER_ID"]:
            require(name)
        destination.mkdir(parents=True)
        installed = Path.home() / "Library/MobileDevice/Provisioning Profiles"
        installed.mkdir(parents=True, exist_ok=True)
        for path, data in profiles.values():
            shutil.copy2(path, installed / (data["UUID"] + ".mobileprovision"))
        run("xcodegen", "generate", "--spec", ROOT / "ios/project.yml", "--project", ROOT / "ios")
        archive = destination / "AppleBridge.xcarchive"
        run("xcodebuild", "-quiet", "-project", ROOT / "ios/AppleBridge.xcodeproj", "-scheme", "AppleBridge", "-configuration", "Release", "-destination", "generic/platform=iOS", "-archivePath", archive, "-derivedDataPath", ROOT / ".build/ios/DerivedData", f"DEVELOPMENT_TEAM={team}", "CODE_SIGN_STYLE=Manual", f"CODE_SIGN_IDENTITY={identity}", f"OTHER_CODE_SIGN_FLAGS=--keychain {keychain}", "APPLE_BRIDGE_APNS_ENVIRONMENT=production", f"MARKETING_VERSION={marketing}", f"CURRENT_PROJECT_VERSION={build}", *[f"{k}={v}" for k, v in settings.items()], "archive", env=xcode_environment())
        print("R1-TESTFLIGHT-ARCHIVE passed=true", flush=True)
        options = dict(method="app-store-connect", destination="export", teamID=team, signingStyle="manual", signingCertificate=identity, provisioningProfiles={bundle: data["Name"] for bundle, (_, data) in profiles.items()}, manageAppVersionAndBuildNumber=False, testFlightInternalTestingOnly=True)
        export = destination / "ExportOptions.plist"
        export.write_bytes(plistlib.dumps(options))
        run("xcodebuild", "-exportArchive", "-archivePath", archive, "-exportOptionsPlist", export, "-exportPath", destination / "export", env=xcode_environment())
        ipa = destination / "export/AppleBridge.ipa"
        if not ipa.is_file():
            raise ValueError("Xcode did not export AppleBridge.ipa. Inspect the export log before retrying.")
        validate_internal_ipa(ipa)
        print("R1-TESTFLIGHT-EXPORT passed=true", flush=True)
        if args.upload_internal:
            from tempfile import TemporaryDirectory
            with TemporaryDirectory(prefix="ab-test-asc-") as temp:
                private = Path(temp) / f"AuthKey_{require('ASC_KEY_ID')}.p8"
                private.symlink_to(Path(require("ASC_KEY_PATH")).expanduser().resolve())
                run("xcrun", "altool", "--upload-app", "--type", "ios", "--file", ipa, "--apiKey", require("ASC_KEY_ID"), "--apiIssuer", require("ASC_ISSUER_ID"), env=dict(os.environ, API_PRIVATE_KEYS_DIR=temp))
        print(f"R1-TESTFLIGHT passed=true uploaded={str(args.upload_internal).lower()}")
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"TestFlight preparation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
