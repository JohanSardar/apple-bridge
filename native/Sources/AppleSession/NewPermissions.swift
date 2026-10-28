import Foundation
import MessagesSource
import CoreLocation
import AVFoundation
import CoreGraphics
import Speech
import Intents
import Photos
import CoreBluetooth
import UserNotifications
import AppKit
import Carbon

@MainActor
public enum NewPermissions {
    public static let automation = ["automation_mail": "com.apple.mail", "automation_music": "com.apple.Music", "automation_safari": "com.apple.Safari", "automation_shortcuts_events": "com.apple.shortcuts.events"]
    public static let names = ["location", "camera", "microphone", "screen_recording", "speech_recognition", "focus", "photos", "bluetooth", "notifications"] + automation.keys.sorted()

    public static func status() async -> [String: String] {
        var result = Dictionary(uniqueKeysWithValues: names.map { ($0, "unknown") })
        func state(_ value: Int, authorized: [Int]) -> String {
            value == 0 ? "not_determined" : authorized.contains(value) ? "authorized" : [1, 2].contains(value) ? "denied" : "unknown"
        }
        result["location"] = state(Int(CLLocationManager().authorizationStatus.rawValue), authorized: [3, 4])
        result["camera"] = state(AVCaptureDevice.authorizationStatus(for: .video).rawValue, authorized: [3])
        result["microphone"] = state(AVCaptureDevice.authorizationStatus(for: .audio).rawValue, authorized: [3])
        result["speech_recognition"] = state(SFSpeechRecognizer.authorizationStatus().rawValue, authorized: [3])
        result["photos"] = state(PHPhotoLibrary.authorizationStatus(for: .readWrite).rawValue, authorized: [3, 4])
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
    private static var location: CLLocationManager?
    private static var bluetooth: CBCentralManager?

    private static func call(_ name: String, adapters: [String: RequestAdapter]?, native: RequestAdapter) async throws {
        requestAPICalls += 1
        if let adapters {
            guard let adapter = adapters[name] else { throw HelperCommandFailure.unknownCommand }
            try await adapter()
        } else {
            try await native()
        }
    }

    public static func requestNative(_ name: String, adapters: [String: RequestAdapter]? = nil) async throws {
        switch name {
        case "location":
            try await call("location", adapters: adapters) {
                location = CLLocationManager()
                location?.requestWhenInUseAuthorization()
                for _ in 0..<240 {
                    if location?.authorizationStatus != .notDetermined { break }
                    try await Task.sleep(for: .milliseconds(250))
                }
            }
        case "camera":
            try await call("camera", adapters: adapters) { _ = await AVCaptureDevice.requestAccess(for: .video) }
        case "microphone":
            try await call("microphone", adapters: adapters) { _ = await AVCaptureDevice.requestAccess(for: .audio) }
        case "screen_recording":
            try await call("screen_recording", adapters: adapters) { _ = CGRequestScreenCaptureAccess() }
        case "speech_recognition":
            try await call("speech_recognition", adapters: adapters) {
                await withCheckedContinuation { continuation in
                    SFSpeechRecognizer.requestAuthorization { _ in continuation.resume() }
                }
            }
        case "focus":
            try await call("focus", adapters: adapters) { _ = await INFocusStatusCenter.default.requestAuthorization() }
        case "photos":
            try await call("photos", adapters: adapters) { _ = await PHPhotoLibrary.requestAuthorization(for: .readWrite) }
        case "bluetooth":
            try await call("bluetooth", adapters: adapters) {
                bluetooth = CBCentralManager(delegate: nil, queue: nil, options: [CBCentralManagerOptionShowPowerAlertKey: false])
                for _ in 0..<240 {
                    if CBManager.authorization != .notDetermined { break }
                    try await Task.sleep(for: .milliseconds(250))
                }
            }
        case "notifications":
            try await call("notifications", adapters: adapters) { _ = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) }
        default:
            guard let bundle = automation[name] else { throw HelperCommandFailure.unknownCommand }
            try await call(name, adapters: adapters) {
                _ = await Task.detached {
                    let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                    return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
                }.value
            }
        }
    }
}
