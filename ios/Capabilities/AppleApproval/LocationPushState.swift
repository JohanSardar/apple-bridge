import Foundation

struct LocationPushSnapshot: Codable {
    let latitude: Double
    let longitude: Double
    let accuracy: Double
    let date: Date
}

enum LocationPushState {
    static var group: String? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "ABLocationPushAppGroup") as? String, !group.isEmpty else { return nil }
        return group
    }
    static var enabled: Bool {
        guard let group else { return false }
        return UserDefaults(suiteName: group)?.bool(forKey: "locationPush.enabled") == true
    }
    static var snapshotURL: URL? {
        guard let group else { return nil }
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?.appendingPathComponent("location-push.json")
    }
    static func setEnabled(_ enabled: Bool) {
        guard let group else { return }
        UserDefaults(suiteName: group)?.set(enabled, forKey: "locationPush.enabled")
        if !enabled, let url = snapshotURL { try? FileManager.default.removeItem(at: url) }
    }
}
