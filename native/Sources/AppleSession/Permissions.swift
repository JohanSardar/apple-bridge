import Foundation
import AppKit
import EventKit
import Contacts
@preconcurrency import ApplicationServices
import Carbon
import MessagesSource

public enum PermissionState: String {
    case granted, denied, notDetermined = "not_determined", targetNotRunning = "target_not_running", unknown
}

public final class PermissionReport: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = Dictionary(uniqueKeysWithValues: ["full_disk_access", "automation_messages", "automation_system_events", "automation_reminders", "accessibility", "contacts", "calendar", "reminders"].map { ($0, "unknown") })

    public func note(_ name: String, _ value: String) {
        lock.lock()
        defer { lock.unlock() }
        values[name] = value
    }

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
        for (name, bundle) in [("automation_messages", "com.apple.MobileSMS"), ("automation_system_events", "com.apple.systemevents"), ("automation_reminders", "com.apple.reminders")] {
            let report = self.report
            let done = DispatchSemaphore(value: 0)
            DispatchQueue.global().async { @Sendable in
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                let status = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
                report.set(name, status == noErr ? .granted : status == -1744 ? .notDetermined : status == -1743 ? .denied : status == -600 ? .targetNotRunning : .unknown)
                done.signal()
            }
            if done.wait(timeout: .now() + 3) == .timedOut {
                report.set(name, .unknown)
            }
        }
    }

    public func authorizeReminders() async {
        let status = await Task.detached { @Sendable in
            let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.reminders")
            return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
        }.value
        report.set("automation_reminders", status == noErr ? .granted : status == -1744 ? .notDetermined : status == -1743 ? .denied : status == -600 ? .targetNotRunning : .unknown)
    }

    public func onboard() async {
        refresh()
        if report.snapshot().values.allSatisfy({ $0 == PermissionState.granted.rawValue }) {
            return
        }
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate()
        refresh()
        let states = report.snapshot()
        if states["calendar"] != PermissionState.granted.rawValue {
            do {
                let granted = try await events.requestFullAccessToEvents()
                report.note("calendar_request", granted ? "granted" : "refused")
            } catch {
                report.note("calendar_request", "error \((error as NSError).domain) \((error as NSError).code)")
            }
        }
        if states["reminders"] != PermissionState.granted.rawValue {
            do {
                let granted = try await events.requestFullAccessToReminders()
                report.note("reminders_request", granted ? "granted" : "refused")
            } catch {
                report.note("reminders_request", "error \((error as NSError).domain) \((error as NSError).code)")
            }
        }
        if states["contacts"] != PermissionState.granted.rawValue {
            do {
                let granted = try await contacts.requestAccess(for: .contacts)
                report.note("contacts_request", granted ? "granted" : "refused")
            } catch {
                report.note("contacts_request", "error \((error as NSError).domain) \((error as NSError).code)")
            }
        }
        if states["accessibility"] != PermissionState.granted.rawValue {
            let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
            _ = AXIsProcessTrustedWithOptions(options)
        }
        if states["full_disk_access"] != PermissionState.granted.rawValue, let pane = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
            NSWorkspace.shared.open(pane)
        }
        if states["automation_messages"] != PermissionState.granted.rawValue {
            requestAutomation("com.apple.MobileSMS")
        }
        if states["automation_system_events"] != PermissionState.granted.rawValue {
            requestAutomation("com.apple.systemevents")
        }
        refresh()
        NSApplication.shared.setActivationPolicy(.accessory)
    }

    private func requestAutomation(_ bundle: String) {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { application, _ in
            guard application != nil else { return }
            DispatchQueue.global().async { @Sendable in
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
