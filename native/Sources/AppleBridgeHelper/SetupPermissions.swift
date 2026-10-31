import AppKit
import AppleSession
import Contacts
import EventKit
import Carbon
import Photos
import ApplicationServices
import SQLite3

@MainActor
final class SetupPermissions {
    func status() async -> [String: String] {
        var result = await NewPermissions.status()
        result["accessibility"] = AXIsProcessTrusted() ? "granted" : "denied"
        let clipboard = NSPasteboard.general.accessBehavior
        result["clipboard"] = clipboard == .alwaysAllow ? "granted" : clipboard == .alwaysDeny ? "denied" : "not_determined"
        let contact = CNContactStore.authorizationStatus(for: .contacts)
        result["contacts"] = contact == .authorized ? "granted" : contact == .limited ? "limited" : contact == .notDetermined ? "not_determined" : "denied"
        for (name, type) in [("calendar", EKEntityType.event), ("reminders", EKEntityType.reminder)] {
            let state = EKEventStore.authorizationStatus(for: type)
            result[name] = state == .fullAccess ? "granted" : state == .notDetermined ? "not_determined" : "denied"
        }
        if PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited { result["photos"] = "limited" }
        do {
            let file = try FileHandle(forReadingFrom: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/com.apple.TCC/TCC.db"))
            try file.close()
            result["full_disk_access"] = "granted"
        } catch {
            let failure = error as NSError
            result["full_disk_access"] = failure.code == NSFileReadNoPermissionError || (failure.userInfo[NSUnderlyingErrorKey] as? NSError)?.code == 1 || (failure.userInfo[NSUnderlyingErrorKey] as? NSError)?.code == 13 ? "denied" : "unknown"
        }
        for permission in SetupPermission.all where permission.bundle != nil {
            let bundle = permission.bundle!
            let code = await Task.detached {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
            }.value
            result[permission.id] = code == noErr ? "granted" : code == -1744 ? "not_determined" : code == -1743 ? "denied" : code == -600 ? "target_not_running" : "unknown"
        }
        return result
    }
    func compareTCC(_ statuses: [String: String]) -> (matches: Bool, count: Int) {
        let services = ["kTCCServiceAddressBook": "contacts", "kTCCServiceCalendar": "calendar", "kTCCServiceCalendarFullAccess": "calendar", "kTCCServiceReminders": "reminders", "kTCCServicePhotos": "photos", "kTCCServiceCamera": "camera", "kTCCServiceMicrophone": "microphone", "kTCCServiceSpeechRecognition": "speech_recognition", "kTCCServiceBluetoothAlways": "bluetooth", "kTCCServiceAccessibility": "accessibility", "kTCCServiceScreenCapture": "screen_recording", "kTCCServiceSystemPolicyAllFiles": "full_disk_access"]
        var count = 0
        var matches = true
        let files = [FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/com.apple.TCC/TCC.db").path, "/Library/Application Support/com.apple.TCC/TCC.db"]
        for path in files {
            var db: OpaquePointer?
            guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { sqlite3_close(db); continue }
            defer { sqlite3_close(db) }
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT service, auth_value, indirect_object_identifier FROM access WHERE client = ?", -1, &statement, nil) == SQLITE_OK else { continue }
            defer { sqlite3_finalize(statement) }
            let identity = Bundle.main.bundleIdentifier ?? "dev.applebridge.helper"
            _ = identity.withCString { sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
            while sqlite3_step(statement) == SQLITE_ROW {
                guard let rawService = sqlite3_column_text(statement, 0) else { continue }
                let service = String(cString: rawService)
                let value = sqlite3_column_int(statement, 1)
                var name = services[service]
                if service == "kTCCServiceAppleEvents", let target = sqlite3_column_text(statement, 2) {
                    name = SetupPermission.all.first { $0.bundle == String(cString: target) }?.id
                }
                guard let name, let status = statuses[name], !["unknown", "target_not_running"].contains(status), value != 1 else { continue }
                let allowed = ["authorized", "granted", "limited"].contains(status)
                matches = matches && (allowed == (value == 2 || value == 3))
                count += 1
            }
        }
        return (matches, count)
    }
    func request(_ permission: SetupPermission) async throws {
        if let bundle = permission.bundle {
            if NSRunningApplication.runningApplications(withBundleIdentifier: bundle).isEmpty,
               let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) {
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = false
                _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            }
            _ = await Task.detached {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
            }.value
            return
        }
        switch permission.id {
        case "contacts": _ = try await CNContactStore().requestAccess(for: .contacts)
        case "calendar": _ = try await EKEventStore().requestFullAccessToEvents()
        case "reminders": _ = try await EKEventStore().requestFullAccessToReminders()
        case "clipboard": _ = NSPasteboard.general.string(forType: .string)
        case "full_disk_access", "accessibility": break
        default: try await NewPermissions.requestNative(permission.id)
        }
    }
}
