#if LAB
import Foundation
import AppKit
import Carbon
import CoreGraphics
import SourceSupport

@MainActor
enum QualificationPreflight {
    static func run(_ scenario: String) async -> JSON {
        if scenario == "CAP-SIM" {
            return .object(["grant_passed": .bool(CGPreflightScreenCaptureAccess()), "screen_capture_preflight": .bool(CGPreflightScreenCaptureAccess())])
        }
        let bundle: String
        switch scenario {
        case "WEB-REM-PARITY": bundle = "com.apple.reminders"
        case "WEB-CAL-PARITY": bundle = "com.apple.iCal"
        default: bundle = "com.apple.mail"
        }
        var opened: NSRunningApplication?
        defer { if let opened { _ = opened.terminate() } }
        if NSRunningApplication.runningApplications(withBundleIdentifier: bundle).isEmpty,
           let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = false
            configuration.addsToRecentItems = false
            opened = try? await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
        }
        let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
        let code = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        return .object(["grant_passed": .bool(code == noErr), "automation_code": .number(Double(code))])
    }
}
#endif
