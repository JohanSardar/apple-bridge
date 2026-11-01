import Foundation

enum ApprovalInbox {
    static func stored(_ id: String, defaults: UserDefaults = .standard) -> PhoneJob? {
        guard let data = defaults.data(forKey: "approval." + id) else { return nil }
        return try? JSONDecoder().decode(PhoneJob.self, from: data)
    }

    static func pending(defaults: UserDefaults = .standard) -> [PhoneJob] {
        defaults.dictionaryRepresentation().keys.filter { $0.hasPrefix("approval.") }.compactMap {
            stored(String($0.dropFirst("approval.".count)), defaults: defaults)
        }
    }

    static func remove(_ id: String, defaults: UserDefaults = .standard) {
        defaults.removeObject(forKey: "approval." + id)
    }
}
