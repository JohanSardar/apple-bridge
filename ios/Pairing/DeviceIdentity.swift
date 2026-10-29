import CryptoKit
import Foundation
import Security

struct DeviceIdentity: Codable {
    let signing: Data
    let encryption: Data
    var deviceID: String?
    var address: String?
    var fingerprint: String?
    var remote: Bool?
    var gatewayKey: Data?
    var pushToken: String?

    static func load() throws -> DeviceIdentity {
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query.merging([kSecReturnData as String: true], uniquingKeysWith: { _, new in new }) as CFDictionary, &result)
        if status == errSecSuccess, let data = result as? Data {
            return try JSONDecoder().decode(DeviceIdentity.self, from: data)
        }
        guard status == errSecItemNotFound else { throw IdentityError.keychain(status) }
        let identity = DeviceIdentity(signing: Curve25519.Signing.PrivateKey().rawRepresentation, encryption: Curve25519.KeyAgreement.PrivateKey().rawRepresentation)
        try identity.save()
        return identity
    }

    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "com.johansardar.applebridge.ios.identity", kSecAttrAccount as String: "device"]
    }

    func save() throws {
        let data = try JSONEncoder().encode(self)
        let attrs: [String: Any] = [kSecValueData as String: data, kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        let status = SecItemUpdate(Self.query as CFDictionary, attrs as CFDictionary)
        if status == errSecItemNotFound {
            let inserted = SecItemAdd(Self.query.merging(attrs, uniquingKeysWith: { _, new in new }) as CFDictionary, nil)
            guard inserted == errSecSuccess else { throw IdentityError.keychain(inserted) }
        } else if status != errSecSuccess { throw IdentityError.keychain(status) }
    }
}

enum IdentityError: Error {
    case keychain(OSStatus), invalidCode, requestFailed
}

extension IdentityError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .keychain(let status): return "Keychain error \(status)"
        case .invalidCode: return "Invalid or expired pairing code"
        case .requestFailed: return "Gateway rejected the request"
        }
    }
}
