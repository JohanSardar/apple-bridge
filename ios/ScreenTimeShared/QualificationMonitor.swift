#if DEBUG
import Foundation
import DeviceActivity
import ManagedSettings

enum QualificationMonitor {
    static func urls(_ run: String) throws -> (URL, URL) {
        guard UUID(uuidString: run) != nil, let root = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: ScreenTimeState.group) else { throw CocoaError(.fileWriteNoPermission) }
        return (root.appendingPathComponent("ab-test-monitor-" + run + ".active"), root.appendingPathComponent("ab-test-monitor-" + run + ".started"))
    }

    static func prepare(_ run: String) throws {
        try remove(run)
        ScreenTimeState.defaults.set(run, forKey: "ab-test-active-monitor")
        let (active, _) = try urls(run)
        try Data().write(to: active, options: .atomic)
    }

    static func record(_ name: String) {
        guard name.hasPrefix("[ab-test] "), let (active, result) = try? urls(String(name.dropFirst(10))), FileManager.default.fileExists(atPath: active.path) else { return }
        try? Data("1".utf8).write(to: result, options: .atomic)
        if !FileManager.default.fileExists(atPath: active.path) { try? FileManager.default.removeItem(at: result) }
    }

    static func started(_ run: String) -> Bool {
        guard let (_, result) = try? urls(run) else { return false }
        return (try? Data(contentsOf: result)) == Data("1".utf8)
    }

    @MainActor
    static func recover() throws -> Int {
        guard let run = ScreenTimeState.defaults.string(forKey: "ab-test-active-monitor") else { return 0 }
        _ = try urls(run)
        DeviceActivityCenter().stopMonitoring([.init("[ab-test] " + run)])
        ManagedSettingsStore(named: .init("[ab-test] " + run)).clearAllSettings()
        try remove(run)
        return 1
    }

    static func remove(_ run: String) throws {
        let (active, result) = try urls(run)
        for url in [active, result] where FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
        if ScreenTimeState.defaults.string(forKey: "ab-test-active-monitor") == run { ScreenTimeState.defaults.removeObject(forKey: "ab-test-active-monitor") }
    }
}
#endif
