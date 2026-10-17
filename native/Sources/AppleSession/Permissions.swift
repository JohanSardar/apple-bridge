import Foundation
import AppKit
import EventKit
import Contacts
@preconcurrency import ApplicationServices
import Carbon
import MessagesSource

public enum PermissionState: String {
    case granted, denied, notDetermined = "not_determined", unknown
}

public final class PermissionReport: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = Dictionary(uniqueKeysWithValues: ["full_disk_access", "automation_messages", "automation_system_events", "accessibility", "contacts", "calendar", "reminders"].map { ($0, "unknown") })

    public func set(_ name: String, _ state: PermissionState) {
        lock.lock()
        defer { lock.unlock() }
        values[name] = state.rawValue
    }

    public func snapshot() -> [String: String] {
        lock.lock()
        defer { lock.unlock() }
        return values
    }
}

@MainActor
public final class PermissionSession {
    private let events = EKEventStore()
    private let contacts = CNContactStore()
    public let report = PermissionReport()

    public init() {}

    public func refresh() {
        do {
            let file = try FileHandle(forReadingFrom: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db"))
            try file.close()
            report.set("full_disk_access", .granted)
        } catch {
            let failure = error as NSError
            report.set("full_disk_access", failure.code == NSFileReadNoPermissionError || (failure.userInfo[NSUnderlyingErrorKey] as? NSError)?.code == 1 || (failure.userInfo[NSUnderlyingErrorKey] as? NSError)?.code == 13 ? .denied : .unknown)
        }
        report.set("accessibility", AXIsProcessTrusted() ? .granted : .denied)
        let contactState = CNContactStore.authorizationStatus(for: .contacts)
        report.set("contacts", contactState == .authorized ? .granted : contactState == .notDetermined ? .notDetermined : contactState == .denied || contactState == .restricted ? .denied : .unknown)
        for (name, type) in [("calendar", EKEntityType.event), ("reminders", EKEntityType.reminder)] {
            let state = EKEventStore.authorizationStatus(for: type)
            report.set(name, state == .fullAccess ? .granted : state == .notDetermined ? .notDetermined : state == .denied || state == .restricted || state == .writeOnly ? .denied : .unknown)
        }
        for (name, bundle) in [("automation_messages", "com.apple.MobileSMS"), ("automation_system_events", "com.apple.systemevents")] {
            let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
            let status = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
            report.set(name, status == noErr ? .granted : status == -1744 ? .notDetermined : status == -1743 ? .denied : .unknown)
        }
    }

    public func onboard() {
        events.requestFullAccessToEvents { _, _ in }
        events.requestFullAccessToReminders { _, _ in }
        contacts.requestAccess(for: .contacts) { _, _ in }
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
        requestAutomation("com.apple.MobileSMS")
        requestAutomation("com.apple.systemevents")
    }

    private func requestAutomation(_ bundle: String) {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { application, _ in
            guard application != nil else { return }
            DispatchQueue.global().async {
                Self.readVersion(bundle)
            }
        }
    }

    nonisolated private static func readVersion(_ bundle: String) {
        let property = NSAppleEventDescriptor.record()
        property.setDescriptor(NSAppleEventDescriptor(typeCode: OSType(cProperty)), forKeyword: AEKeyword(keyAEDesiredClass))
        property.setDescriptor(NSAppleEventDescriptor(enumCode: OSType(formPropertyID)), forKeyword: AEKeyword(keyAEKeyForm))
        property.setDescriptor(NSAppleEventDescriptor(typeCode: OSType(pVersion)), forKeyword: AEKeyword(keyAEKeyData))
        property.setDescriptor(NSAppleEventDescriptor.null(), forKeyword: AEKeyword(keyAEContainer))
        guard let object = property.coerce(toDescriptorType: DescType(typeObjectSpecifier)) else { return }
        let event = NSAppleEventDescriptor(eventClass: AEEventClass(kAECoreSuite), eventID: AEEventID(kAEGetData), targetDescriptor: NSAppleEventDescriptor(bundleIdentifier: bundle), returnID: AEReturnID(kAutoGenerateReturnID), transactionID: AETransactionID(kAnyTransactionID))
        event.setParam(object, forKeyword: AEKeyword(keyDirectObject))
        _ = try? event.sendEvent(options: [.waitForReply, .canInteract], timeout: 3)
    }
}
