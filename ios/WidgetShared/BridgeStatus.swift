import Foundation
import Security

struct BridgeStatus: Codable {
    let status: String
    let updated: Date

    private static var query: [String: Any] {
        var value: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                   kSecAttrService as String: "apple-bridge-widget-status",
                                   kSecAttrAccount as String: "status"]
        if let group = Bundle.main.object(forInfoDictionaryKey: "ABWidgetKeychainGroup") as? String {
            value[kSecAttrAccessGroup as String] = group
        }
        return value
    }

    static func load() -> BridgeStatus {
        var query = Self.query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data,
              let value = try? JSONDecoder().decode(Self.self, from: data) else {
            return Self(status: "Open Apple Bridge", updated: .distantPast)
        }
        return value
    }

    func save() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        let attributes: [String: Any] = [kSecValueData as String: data,
                                       kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        if SecItemUpdate(Self.query as CFDictionary, attributes as CFDictionary) == errSecItemNotFound {
            var item = Self.query
            item.merge(attributes, uniquingKeysWith: { _, new in new })
            SecItemAdd(item as CFDictionary, nil)
        }
    }
}
