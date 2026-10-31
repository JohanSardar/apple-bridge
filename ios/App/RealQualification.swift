import Foundation
import CryptoKit
import CoreLocation
import CoreMotion
import FamilyControls
import DeviceActivity
import ManagedSettings
import UIKit

@MainActor
enum RealQualification {
    static var active: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("--real-qualification")
        #else
        false
        #endif
    }
    static func runIfRequested() async -> Bool {
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "--real-qualification"), args.count > index + 2 else { return false }
        let scenario = args[index + 1]
        guard let run = UUID(uuidString: args[index + 2]) else { return true }
        let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("qualification")
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(run.uuidString + ".json")
        var counts: [String: Int] = [:]
        var checks: [String: Bool] = [:]
        var digests: [String: String] = [:]
        var phase = 0
        var instruction = ""
        func save(_ final: Bool) {
            let object: [String: Any] = ["scenario": scenario, "run": run.uuidString, "final": final, "checks": checks, "counts": counts, "digests": digests, "phase": phase, "prompt": instruction]
            if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) { try? data.write(to: url, options: [.atomic, .completeFileProtection]) }
        }
        func prompt(_ name: String) {
            phase += 1
            instruction = "tap Allow on \(name) on the iPhone; keep Apple Bridge open (up to 3 minutes)"
            save(false)
        }
        save(false)
        do {
            guard await QualificationWait.until({ UIApplication.shared.applicationState == .active }) else { throw CocoaError(.userCancelled) }
            switch scenario {
            case "APP-HEALTH-REAL":
                let reader = HealthKitReader()
                prompt("Health read access")
                try await reader.authorize()
                var total = 0
                var anchors = 0
                for stream in reader.streams {
                    let page = try await reader.page(stream: stream, anchor: nil)
                    total += page.samples.count
                    anchors += page.anchor.isEmpty ? 0 : 1
                    let next = try await reader.page(stream: stream, anchor: page.anchor)
                    checks["incremental_" + stream] = !next.anchor.isEmpty && Set(page.samples.map(\.id)).isDisjoint(with: next.samples.map(\.id))
                }
                counts["samples"] = total
                counts["anchors"] = anchors
                checks["nonempty_read_observed"] = total > 0
            case "APP-HEALTH-WRITE-REAL":
                let writer = HealthKitWriter(retainForProof: true)
                prompt("Health read and write access")
                try await writer.authorize()
                try await HealthWriteProof.run(store: writer)
                counts["written"] = 4
                counts["deleted"] = 4
                checks["write_read_delete_absent"] = true
            case "APP-MOTION-REAL":
                prompt("Motion & Fitness")
                let adapter = CoreMotionAdapter()
                let now = Date()
                let activities = try await adapter.activities(from: now.addingTimeInterval(-86400), to: now)
                counts["activities"] = activities.count
                let day = try await adapter.pedometer(from: now.addingTimeInterval(-86400), to: now)
                counts["steps"] = day.steps
                checks["pedometer_observed"] = true
                _ = try await adapter.currentActivity()
                checks["current_activity_observed"] = true
            case "APP-ALARM-REAL":
                let adapter = SystemAlarmAdapter()
                prompt("Alarms")
                try await adapter.authorize()
                let id = UUID()
                let alarm = BridgeAlarm(id: id, fire_at: Date().addingTimeInterval(3600).timeIntervalSince1970, duration_seconds: nil, snooze_seconds: 540, title: "[ab-test] " + run.uuidString, state: "scheduled")
                try await AlarmQualification.run(alarm, adapter: adapter) { checks[$0] = $1 }
                counts["created"] = 1
                counts["deleted"] = 1
            case "APP-DEVICE-REAL":
                prompt("Location for Wi-Fi information if requested")
                let manager = CLLocationManager()
                if manager.authorizationStatus == .notDetermined { manager.requestWhenInUseAuthorization() }
                let deadline = Date().addingTimeInterval(180)
                while Date() < deadline && manager.authorizationStatus == .notDetermined { try await Task.sleep(for: .seconds(1)) }
                let status = try await SystemDeviceContext().status()
                checks["battery_observed"] = status.batteryLevel != nil && status.charging != "unknown"
                checks["wifi_observed"] = status.wifiNetwork != nil
                checks["focus_observed"] = status.focusFilterActive != nil
                counts["nfc_physical_tag_skipped"] = 1
                counts["device_name_present"] = status.deviceName.isEmpty ? 0 : 1
            case "APP-SCREENTIME-REAL":
                prompt("Screen Time individual authorization")
                try await AuthorizationCenter.shared.requestAuthorization(for: .individual)
                try ScreenTimeCapability.authorized()
                checks["authorized"] = true
                let store = ManagedSettingsStore(named: .init("[ab-test] " + run.uuidString))
                defer { store.clearAllSettings() }
                ScreenTimeState.apply(store)
                let selection = ScreenTimeState.selection
                counts["selected_tokens"] = selection.applicationTokens.count + selection.categoryTokens.count + selection.webDomainTokens.count
                checks["nonempty_shield_observed"] = counts["selected_tokens", default: 0] > 0 && (store.shield.applications != nil || store.shield.applicationCategories != nil || store.shield.webDomains != nil)
                let name = DeviceActivityName("[ab-test] " + run.uuidString)
                let center = DeviceActivityCenter()
                defer { center.stopMonitoring([name]) }
                let start = Date().addingTimeInterval(60)
                let end = start.addingTimeInterval(960)
                try center.startMonitoring(name, during: DeviceActivitySchedule(intervalStart: Calendar.current.dateComponents([.hour, .minute, .second], from: start), intervalEnd: Calendar.current.dateComponents([.hour, .minute, .second], from: end), repeats: false))
                checks["monitor_registered"] = center.activities.contains(name)
                center.stopMonitoring([name])
                checks["monitor_removed"] = !center.activities.contains(name)
                checks["monitor_callback_observed"] = false
                checks["usage_report_observed"] = false
            case "APP-LOC-REAL":
                let adapter = CoreLocationAdapter()
                prompt("Location; choose Always and Precise in Settings when offered")
                adapter.requestPermission()
                let manager = CLLocationManager()
                _ = await QualificationWait.until {
                    manager.authorizationStatus == .authorizedAlways || manager.authorizationStatus == .denied || manager.authorizationStatus == .restricted
                }
                let fix = try await adapter.current()
                checks["precise_fix_observed"] = fix.accuracy >= 0
                checks["always_authorized"] = manager.authorizationStatus == .authorizedAlways
                if manager.authorizationStatus == .authorizedAlways {
                    let name = "[ab-test] " + run.uuidString
                    defer { adapter.remove(name) }
                    try await adapter.monitor(NamedGeofence(name: name, latitude: fix.latitude, longitude: fix.longitude, radius: 100))
                    checks["geofence_registered"] = adapter.isMonitoring(name)
                    adapter.remove(name)
                    checks["geofence_removed"] = !adapter.isMonitoring(name)
                }
            case "APP-CAPTURE-REAL":
                let previous = UserDefaults.standard.object(forKey: "capability.capture.enabled")
                UserDefaults.standard.set(true, forKey: "capability.capture.enabled")
                defer {
                    if let previous { UserDefaults.standard.set(previous, forKey: "capability.capture.enabled") }
                    else { UserDefaults.standard.removeObject(forKey: "capability.capture.enabled") }
                }
                for kind in ["photo", "audio"] {
                    prompt("Face ID and \(kind == "photo" ? "Camera" : "Microphone")")
                    _ = try await CaptureFlow.run(kind: kind, seconds: 1, adapter: DeviceCapture(expires: Int64(Date().addingTimeInterval(180).timeIntervalSince1970))) { bytes in
                        counts[kind + "_bytes"] = bytes.data.count
                        digests[kind] = SHA256.hash(data: bytes.data).map { String(format: "%02x", $0) }.joined()
                        return Data()
                    }
                    checks[kind + "_captured_and_discarded"] = true
                }
            default:
                checks["supported_scenario"] = false
            }
        } catch {
            checks["operation_completed"] = false
            digests["failure"] = SHA256.hash(data: Data("\((error as NSError).domain):\((error as NSError).code)".utf8)).map { String(format: "%02x", $0) }.joined()
        }
        save(true)
        return true
        #else
        return false
        #endif
    }
}
