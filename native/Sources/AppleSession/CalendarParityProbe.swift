#if LAB
import Foundation
import AppKit
import Carbon
import Darwin
import ApplicationServices
import SourceSupport

@MainActor
enum CalendarParityProbe {
    static func run() async throws -> JSON {
        var buildSize = 0
        sysctlbyname("kern.osversion", nil, &buildSize, nil, 0)
        var build = [CChar](repeating: 0, count: max(1, buildSize))
        sysctlbyname("kern.osversion", &build, &buildSize, nil, 0)
        let osBuild = String(decoding: build.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.iCal") else { throw SourceFailure.unavailable }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        configuration.addsToRecentItems = false
        let application = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.iCal")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        let trusted = AXIsProcessTrusted()
        let root = AXUIElementCreateApplication(application.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.25)
        var windows: CFTypeRef?
        let result = trusted ? AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &windows) : .apiDisabled
        let count = (windows as? [AXUIElement])?.count ?? 0
        let reason = !trusted ? "calendar_accessibility_permission_missing" : permission == -1744 ? "calendar_navigation_apple_events_consent_pending" : permission == -1743 ? "calendar_navigation_apple_events_denied" : result != .success || count == 0 ? "calendar_accessibility_window_target_missing" : "calendar_exact_event_navigation_and_effect_not_proven"
        let operations = ["calendar.respond_invitation", "calendar.set_travel_time", "calendar.add_attachment", "calendar.remove_attachment"].map { operation in
            JSON.object(["operation": .string(operation), "proven": .bool(false), "reason": .string(reason)])
        }
        return .object(["status": .string("limited"), "os_build": .string(osBuild), "accessibility_trusted": .bool(trusted), "calendar_automation_code": .number(Double(permission)), "calendar_ax_code": .number(Double(result.rawValue)), "calendar_window_count": .number(Double(count)), "operations": .array(operations), "fixture_created": .bool(false), "effects_attempted": .bool(false), "consent_requested": .bool(false)])
    }
}
#endif
