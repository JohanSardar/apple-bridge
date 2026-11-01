import Foundation
import CryptoKit
import CoreLocation
import CoreMotion
import FamilyControls
import DeviceActivity
import ManagedSettings
import UIKit
import SwiftUI

#if DEBUG
@MainActor
@Observable
final class RealQualificationProgress {
    static let shared = RealQualificationProgress()
    var instruction = "Starting [ab-test] qualification"
    var finished = false
    var started = false
    var passed = false
    var screenTime = false
    var screenStage = "selection"
    var selection = FamilyActivitySelection()
    var picker = false
    var shieldConfirmed = false
    var reportAppeared = false
    var reportConfirmed = false
}

struct RealQualificationScreen: View {
    @Bindable private var progress = RealQualificationProgress.shared
    var body: some View {
        NavigationStack {
            List {
                Text(progress.instruction)
                if progress.finished { Text(progress.passed ? "Checks passed. Tagged data cleaned up." : "Checks incomplete. See the qualification result on the Pro.") }
                else { ProgressView("Keep the iPhone unlocked") }
                if progress.screenTime && !progress.finished {
                    Button("Choose test app") { progress.picker = true }.disabled(progress.screenStage != "selection")
                    Button("Confirm shield") { progress.shieldConfirmed = true }.disabled(progress.screenStage != "shield")
                    NavigationLink("Open usage report") {
                        DeviceActivityReport(.init("bridge.usage"), filter: DeviceActivityFilter(segment: .daily(during: Calendar.current.dateInterval(of: .day, for: Date())!), users: .all, devices: .init([.iPhone])))
                            .onAppear { progress.reportAppeared = true }
                    }
                    Button("Confirm usage report") { progress.reportConfirmed = true }.disabled(!progress.reportAppeared || progress.screenStage != "report")
                }
            }
            .navigationTitle("[ab-test] Qualification")
            .familyActivityPicker(isPresented: $progress.picker, selection: $progress.selection)
        }
        .task { _ = await RealQualification.runIfRequested() }
    }
}
#endif

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
        let progress = RealQualificationProgress.shared
        guard !progress.started else { return true }
        progress.started = true
        let scenario = args[index + 1]
        let idleTimer = UIApplication.shared.isIdleTimerDisabled
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = idleTimer }
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
        func prompt(_ name: String, instruction exact: String? = nil) {
            phase += 1
            instruction = "tap Allow on \(name) on the iPhone; keep Apple Bridge open (up to 3 minutes)"
            if name.hasPrefix("Health") { instruction = "tap Turn On All, then Allow in Apple Bridge Health Access" }
            if let exact { instruction = exact }
            RealQualificationProgress.shared.instruction = instruction
            save(false)
        }
        save(false)
        do {
            guard await QualificationWait.until({ UIApplication.shared.applicationState == .active }) else { throw CocoaError(.userCancelled) }
            try await ShellDeliveryQualification.restoreBackup()
            let recovered = try QualificationMonitor.recover()
            if recovered > 0 { counts["recovered_screen_time_runs"] = recovered }
            checks["foreground_active"] = true
            switch scenario {
            case "APP-HEALTH-REAL":
                let reader = HealthKitReader()
                prompt("Health read access")
                try await reader.authorize { prompt("Health medication selection", instruction: "select the medications to share with Apple Bridge, then tap Share if shown") }
                checks["health_authorization_completed"] = true
                save(false)
                var total = 0
                var anchors = 0
                for stream in reader.streams {
                    let page = try await reader.page(stream: stream, anchor: nil)
                    total += page.samples.count
                    anchors += page.anchor.isEmpty ? 0 : 1
                    let next = try await reader.page(stream: stream, anchor: page.anchor)
                    checks["incremental_" + stream] = !next.anchor.isEmpty && Set(page.samples.map(\.id)).isDisjoint(with: next.samples.map(\.id))
                    counts["completed_streams", default: 0] += 1
                    save(false)
                }
                counts["samples"] = total
                counts["anchors"] = anchors
                checks["nonempty_read_observed"] = total > 0
            case "APP-HEALTH-WRITE-REAL":
                let writer = HealthKitWriter(retainForProof: true)
                prompt("Health read and write access")
                try await writer.authorize()
                checks["health_authorization_completed"] = true
                save(false)
                counts["recovered_samples_deleted"] = try await writer.cleanupTaggedSamples()
                do { try await HealthWriteProof.run(store: writer) }
                catch {
                    counts["interrupted_samples_deleted"] = try await writer.cleanupTaggedSamples()
                    checks["resources_cleaned"] = true
                    throw error
                }
                counts["remaining_samples_deleted"] = try await writer.cleanupTaggedSamples()
                checks["resources_cleaned"] = true
                counts["written"] = 4
                counts["deleted"] = 4
                checks["write_read_delete_absent"] = true
            case "APP-MOTION-REAL":
                prompt("Motion & Fitness", instruction: "tap OK on Apple Bridge Motion & Fitness access")
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
                checks["alarm_authorization_completed"] = true
                save(false)
                let id = UUID()
                let alarm = BridgeAlarm(id: id, fire_at: Date().addingTimeInterval(3600).timeIntervalSince1970, duration_seconds: nil, snooze_seconds: 540, title: "[ab-test] " + run.uuidString, state: "scheduled")
                try await AlarmQualification.run(alarm, adapter: adapter) { checks[$0] = $1 }
                counts["created"] = 1
                counts["deleted"] = 1
            case "APP-DEVICE-REAL":
                let batteryMonitoring = UIDevice.current.isBatteryMonitoringEnabled
                defer { UIDevice.current.isBatteryMonitoringEnabled = batteryMonitoring }
                prompt("Location for Wi-Fi information if requested", instruction: "tap Allow While Using App on Apple Bridge Location access; keep Precise Location on")
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
                prompt("Screen Time individual authorization", instruction: "tap Continue, then Allow with Face ID for Apple Bridge Screen Time access")
                counts["authorization_status_before"] = AuthorizationCenter.shared.authorizationStatus.rawValue
                try await AuthorizationCenter.shared.requestAuthorization(for: .individual)
                counts["authorization_status_after"] = AuthorizationCenter.shared.authorizationStatus.rawValue
                try ScreenTimeCapability.authorized()
                checks["authorized"] = true
                ScreenTimeState.defaults.set(run.uuidString, forKey: "ab-test-active-monitor")
                let store = ManagedSettingsStore(named: .init("[ab-test] " + run.uuidString))
                defer { store.clearAllSettings(); try? QualificationMonitor.remove(run.uuidString) }
                let progress = RealQualificationProgress.shared
                progress.screenTime = true
                prompt("test app selection", instruction: "tap Choose test app, select Calculator, then tap Done")
                guard await QualificationWait.until({ !progress.picker && !progress.selection.applicationTokens.isEmpty }) else { throw CocoaError(.userCancelled) }
                let selection = progress.selection
                BridgeShieldPolicy.apply(ManagedShieldAdapter(store: store), apps: selection.applicationTokens, categories: selection.categoryTokens, domains: selection.webDomainTokens)
                counts["selected_tokens"] = selection.applicationTokens.count + selection.categoryTokens.count + selection.webDomainTokens.count
                checks["nonempty_shield_observed"] = counts["selected_tokens", default: 0] > 0 && (store.shield.applications != nil || store.shield.applicationCategories != nil || store.shield.webDomains != nil)
                progress.screenStage = "shield"
                prompt("shield observation", instruction: "open Calculator, return to Apple Bridge after seeing its shield, then tap Confirm shield")
                checks["shield_ui_observed"] = await QualificationWait.until { progress.shieldConfirmed }
                store.clearAllSettings()
                progress.screenStage = "report"
                prompt("usage report", instruction: "tap Open usage report, inspect usage, return to qualification, then tap Confirm usage report")
                checks["usage_report_observed"] = await QualificationWait.until { progress.reportAppeared && progress.reportConfirmed }
                progress.screenStage = "monitor"
                let name = DeviceActivityName("[ab-test] " + run.uuidString)
                let center = DeviceActivityCenter()
                defer { center.stopMonitoring([name]) }
                try QualificationMonitor.prepare(run.uuidString)
                defer { try? QualificationMonitor.remove(run.uuidString) }
                let start = Date().addingTimeInterval(10)
                let end = start.addingTimeInterval(960)
                try center.startMonitoring(name, during: DeviceActivitySchedule(intervalStart: Calendar.current.dateComponents([.hour, .minute, .second], from: start), intervalEnd: Calendar.current.dateComponents([.hour, .minute, .second], from: end), repeats: false))
                checks["monitor_registered"] = center.activities.contains(name)
                prompt("monitor callback", instruction: "keep Apple Bridge open while the tagged Screen Time monitor starts")
                checks["monitor_callback_observed"] = await QualificationWait.until { QualificationMonitor.started(run.uuidString) }
                center.stopMonitoring([name])
                checks["monitor_removed"] = !center.activities.contains(name)
                try QualificationMonitor.remove(run.uuidString)
                checks["resources_cleaned"] = true
            case "APP-LOC-REAL":
                let adapter = CoreLocationAdapter()
                prompt("Location", instruction: "tap Change to Always Allow; if it is not offered, open Settings, Apps, Apple Bridge, Location, select Always and turn Precise Location on, then return to Apple Bridge")
                adapter.requestPermission()
                let manager = CLLocationManager()
                _ = await QualificationWait.until {
                    manager.authorizationStatus == .authorizedAlways || manager.authorizationStatus == .denied || manager.authorizationStatus == .restricted
                }
                for region in manager.monitoredRegions where region.identifier.hasPrefix("ab.location.[ab-test] ") {
                    let name = String(region.identifier.dropFirst("ab.location.".count))
                    guard await LocationQualification.remove(name, remove: adapter.remove, isMonitoring: adapter.isMonitoring) else { throw CocoaError(.fileWriteUnknown) }
                    counts["recovered_geofences_removed", default: 0] += 1
                }
                let fix = try await adapter.current()
                checks["precise_fix_observed"] = fix.accuracy >= 0
                checks["always_authorized"] = manager.authorizationStatus == .authorizedAlways
                if manager.authorizationStatus == .authorizedAlways {
                    let name = "[ab-test] " + run.uuidString
                    defer { adapter.remove(name) }
                    try await adapter.monitor(NamedGeofence(name: name, latitude: fix.latitude, longitude: fix.longitude, radius: 100))
                    checks["geofence_registered"] = adapter.isMonitoring(name)
                    checks["geofence_removed"] = await LocationQualification.remove(name, remove: adapter.remove, isMonitoring: adapter.isMonitoring)
                }
            case "APP-CAPTURE-REAL":
                let previous = UserDefaults.standard.object(forKey: "capability.capture.enabled")
                let key = "capability.capture.enabled"
                let backup = ShellDeliveryBackup(identity: try DeviceIdentity.load(), switches: previous == nil ? [:] : [key: UserDefaults.standard.bool(forKey: key)], absent: previous == nil ? [key] : [], status: BridgeStatus.load(), run: run.uuidString, command: nil)
                var backupURL = ShellDeliveryQualification.directory.appendingPathComponent("ab-real-backup.json")
                try JSONEncoder().encode(backup).write(to: backupURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                var values = URLResourceValues()
                values.isExcludedFromBackup = true
                try backupURL.setResourceValues(values)
                UserDefaults.standard.set(true, forKey: key)
                for kind in ["photo", "audio"] {
                    prompt("Face ID and \(kind == "photo" ? "Camera" : "Microphone")", instruction: "look at the iPhone for Face ID; tap OK on Face ID permission, then OK or Allow on \(kind == "photo" ? "Camera" : "Microphone") access if shown")
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
            if scenario == "APP-SCREENTIME-REAL" { print("[ab-test] Screen Time authorization: \(error)") }
            if let family = error as? FamilyControlsError {
                switch family {
                case .restricted: checks["screen_time_unrestricted"] = false
                case .unavailable: checks["screen_time_service_available"] = false
                case .invalidAccountType: checks["screen_time_account_supported"] = false
                case .invalidArgument: checks["screen_time_request_valid"] = false
                case .authorizationConflict: checks["screen_time_authorization_without_conflict"] = false
                case .authorizationCanceled: checks["screen_time_authorization_confirmed"] = false
                case .networkError: checks["screen_time_network_available"] = false
                case .authenticationMethodUnavailable: checks["screen_time_authentication_available"] = false
                default: checks["screen_time_authorization_granted"] = false
                }
            }
            checks["operation_completed"] = false
            digests["failure"] = SHA256.hash(data: Data("\((error as NSError).domain):\((error as NSError).code)".utf8)).map { String(format: "%02x", $0) }.joined()
        }
        do {
            try await ShellDeliveryQualification.restoreBackup()
            if scenario == "APP-CAPTURE-REAL" { checks["state_restored"] = true }
        } catch { checks["state_restored"] = false }
        save(true)
        RealQualificationProgress.shared.finished = true
        RealQualificationProgress.shared.passed = !checks.isEmpty && checks.values.allSatisfy { $0 }
        return true
        #else
        return false
        #endif
    }
}
