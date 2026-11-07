import Foundation
import Photos
import MessagesSource
import Contacts
import EventKit
import CoreLocation
import AVFoundation
import CoreGraphics
import ScreenCaptureKit
import Speech
import Intents
import CoreBluetooth
import UserNotifications
import AppKit
import Carbon

@MainActor
public enum NewPermissions {
    public static let automation = ["automation_mail": "com.apple.mail", "automation_safari": "com.apple.Safari", "automation_shortcuts_events": "com.apple.shortcuts.events"]
    public static let names = ["photos", "location", "camera", "microphone", "screen_recording", "speech_recognition", "focus", "bluetooth", "notifications"] + automation.keys.sorted()
    public static let levelOne = ["contacts", "calendar", "reminders"]
    public static let requestable = names + levelOne

    public static func status() async -> [String: String] {
        var result = Dictionary(uniqueKeysWithValues: names.map { ($0, "unknown") })
        func state(_ value: Int, authorized: [Int]) -> String {
            value == 0 ? "not_determined" : authorized.contains(value) ? "authorized" : [1, 2].contains(value) ? "denied" : "unknown"
        }
        result["location"] = state(Int(CLLocationManager().authorizationStatus.rawValue), authorized: [3, 4])
        switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
        case .notDetermined: result["photos"] = "not_determined"
        case .authorized: result["photos"] = "authorized"
        case .limited: result["photos"] = "limited"
        case .denied, .restricted: result["photos"] = "denied"
        @unknown default: result["photos"] = "unknown"
        }
        result["camera"] = state(AVCaptureDevice.authorizationStatus(for: .video).rawValue, authorized: [3])
        result["microphone"] = state(AVCaptureDevice.authorizationStatus(for: .audio).rawValue, authorized: [3])
        result["speech_recognition"] = state(SFSpeechRecognizer.authorizationStatus().rawValue, authorized: [3])
        result["bluetooth"] = state(CBManager.authorization.rawValue, authorized: [3])
        result["focus"] = state(INFocusStatusCenter.default.authorizationStatus.rawValue, authorized: [3])
        result["screen_recording"] = CGPreflightScreenCaptureAccess() ? "authorized" : "unknown"
        let notifications = await UNUserNotificationCenter.current().notificationSettings()
        result["notifications"] = notifications.authorizationStatus == .notDetermined ? "not_determined" : notifications.authorizationStatus == .denied ? "denied" : [.authorized, .provisional].contains(notifications.authorizationStatus) ? "authorized" : "unknown"
        for (name, bundle) in automation {
            let code = await Task.detached {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
            }.value
            result[name] = code == noErr ? "authorized" : code == -1744 ? "not_determined" : code == -1743 ? "denied" : "unknown"
        }
        return result
    }

    public static func statusProof() async throws -> [String: String] {
        func windows() throws -> [String: Int] {
            guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { throw ProbeFailure.unavailable }
            var counts: [String: Int] = [:]
            for window in list {
                guard let owner = window[kCGWindowOwnerName as String] as? String else { continue }
                let key = owner.lowercased()
                if ["usernotificationcenter", "tccd", "securityagent", "coreservicesuiagent"].contains(where: { key.contains($0) }) {
                    counts[owner, default: 0] += 1
                }
            }
            return counts
        }
        let before = try windows()
        var peak = before
        var observationFailed = false
        func observe() {
            do {
                for (owner, count) in try windows() { peak[owner] = max(peak[owner, default: 0], count) }
            } catch { observationFailed = true }
        }
        let observer = Task { @MainActor in
            while !Task.isCancelled {
                observe()
                do { try await Task.sleep(for: .milliseconds(10)) } catch { break }
            }
        }
        var result = await status()
        observe()
        observer.cancel()
        await observer.value
        guard !observationFailed else { throw ProbeFailure.unavailable }
        let after = try windows()
        for (owner, count) in after { peak[owner] = max(peak[owner, default: 0], count) }
        func encoded(_ counts: [String: Int]) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: counts, options: [.sortedKeys]), as: UTF8.self)
        }
        result["_request_api_calls"] = String(requestAPICalls)
        result["_windows_before"] = try encoded(before)
        result["_windows_after"] = try encoded(after)
        result["_windows_peak"] = try encoded(peak)
        return result
    }

    public typealias RequestAdapter = @MainActor () async throws -> Void
    public private(set) static var requestAPICalls = 0
    private static var location: LocationRequest?
    private static var bluetooth: CBCentralManager?

    @MainActor
    private final class LocationRequest: NSObject, @preconcurrency CLLocationManagerDelegate {
        let manager = CLLocationManager()
        private(set) var status: CLAuthorizationStatus
        override init() {
            status = manager.authorizationStatus
            super.init()
            manager.delegate = self
        }
        func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
            status = manager.authorizationStatus
        }
    }

    private static func call(_ name: String, adapters: [String: RequestAdapter]?, native: RequestAdapter) async throws {
        requestAPICalls += 1
        if let adapters {
            guard let adapter = adapters[name] else { throw HelperCommandFailure.unknownCommand }
            try await adapter()
        } else {
            try await native()
        }
    }

    public enum RequestOutcome: Sendable, Equatable { case finished, denied, timedOut }
    private enum RequestFailure: Error { case denied }

    private static func requireAccess(_ granted: Bool) throws {
        if !granted { throw RequestFailure.denied }
    }

    private static func isDenial(_ error: any Error) -> Bool {
        if error is RequestFailure { return true }
        let error = error as NSError
        return (error.domain == CNErrorDomain && error.code == CNError.Code.authorizationDenied.rawValue)
            || (error.domain == EKErrorDomain && error.code == EKError.Code.eventStoreNotAuthorized.rawValue)
            || (error.domain == UNErrorDomain && error.code == UNError.Code.notificationsNotAllowed.rawValue)
            || (error.domain == SCStreamErrorDomain && error.code == SCStreamError.Code.userDeclined.rawValue)
    }
    public static let requestDeadline: Duration = .seconds(150)
    public static let watchdogDeadline: Duration = .seconds(155)

    public nonisolated static func armWatchdog(after deadline: Duration, fire: @escaping @Sendable () -> Void) -> DispatchWorkItem {
        let watchdog = DispatchWorkItem(block: fire)
        let (seconds, attoseconds) = deadline.components
        DispatchQueue.global().asyncAfter(deadline: .now() + Double(seconds) + Double(attoseconds) / 1e18, execute: watchdog)
        return watchdog
    }

    private final class FirstOutcome: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<RequestOutcome, Error>?
        init(_ continuation: CheckedContinuation<RequestOutcome, Error>) { self.continuation = continuation }
        func finish(_ result: Result<RequestOutcome, Error>) {
            lock.lock()
            let waiting = continuation
            continuation = nil
            lock.unlock()
            waiting?.resume(with: result)
        }
    }

    public static func request(_ name: String, deadline: Duration = requestDeadline, adapters: [String: RequestAdapter]? = nil) async throws -> RequestOutcome {
        try await withCheckedThrowingContinuation { continuation in
            let first = FirstOutcome(continuation)
            Task { @MainActor in
                do {
                    try await requestNative(name, adapters: adapters)
                    first.finish(.success(.finished))
                } catch {
                    first.finish(isDenial(error) ? .success(.denied) : .failure(error))
                }
            }
            Task.detached {
                try? await Task.sleep(for: deadline)
                first.finish(.success(.timedOut))
            }
        }
    }

    public static func requestNative(_ name: String, adapters: [String: RequestAdapter]? = nil) async throws {
        switch name {
        case "photos":
            try await call("photos", adapters: adapters) {
                let status = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
                try requireAccess(status == .authorized || status == .limited)
            }
        case "contacts":
            try await call("contacts", adapters: adapters) { try requireAccess(try await CNContactStore().requestAccess(for: .contacts)) }
        case "calendar":
            try await call("calendar", adapters: adapters) { try requireAccess(try await EKEventStore().requestFullAccessToEvents()) }
        case "reminders":
            try await call("reminders", adapters: adapters) { try requireAccess(try await EKEventStore().requestFullAccessToReminders()) }
        case "location":
            try await call("location", adapters: adapters) {
                let request = LocationRequest()
                location = request
                request.manager.requestWhenInUseAuthorization()
                while request.status == .notDetermined {
                    try await Task.sleep(for: .milliseconds(250))
                }
                try requireAccess(![.denied, .restricted].contains(request.status))
            }
        case "camera":
            try await call("camera", adapters: adapters) { try requireAccess(await AVCaptureDevice.requestAccess(for: .video)) }
        case "microphone":
            try await call("microphone", adapters: adapters) { try requireAccess(await AVCaptureDevice.requestAccess(for: .audio)) }
        case "screen_recording":
            try await call("screen_recording", adapters: adapters) {
                _ = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            }
        case "speech_recognition":
            try await call("speech_recognition", adapters: adapters) {
                let status = await withCheckedContinuation { continuation in
                    SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0) }
                }
                try requireAccess(![.denied, .restricted].contains(status))
            }
        case "focus":
            try await call("focus", adapters: adapters) { try requireAccess(await INFocusStatusCenter.default.requestAuthorization() == .authorized) }
        case "bluetooth":
            try await call("bluetooth", adapters: adapters) {
                bluetooth = CBCentralManager(delegate: nil, queue: nil, options: [CBCentralManagerOptionShowPowerAlertKey: false])
                for _ in 0..<240 {
                    if CBManager.authorization != .notDetermined { break }
                    try await Task.sleep(for: .milliseconds(250))
                }
                try requireAccess(![.denied, .restricted].contains(CBManager.authorization))
            }
        case "notifications":
            try await call("notifications", adapters: adapters) { try requireAccess(try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) }
        default:
            guard let bundle = automation[name] else { throw HelperCommandFailure.unknownCommand }
            try await call(name, adapters: adapters) {
                let status = await Task.detached {
                    let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                    return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
                }.value
                try requireAccess(status != -1743)
            }
        }
    }
}
