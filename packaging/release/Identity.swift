import Foundation
import Security

func der(_ tag: UInt8, _ value: Data) -> Data {
    var length = value.count
    var bytes: [UInt8] = []
    if length < 128 {
        bytes = [UInt8(length)]
    } else {
        while length > 0 { bytes.insert(UInt8(length & 255), at: 0); length >>= 8 }
        bytes.insert(0x80 | UInt8(bytes.count), at: 0)
    }
    return Data([tag] + bytes) + value
}

func oid(_ value: [UInt8]) -> Data { der(0x06, Data(value)) }

@main
struct ReleaseIdentity {
    static func main() {
        do { try execute() } catch {
            FileHandle.standardError.write(Data("Identity preparation failed: \(error.localizedDescription)\n".utf8))
            exit(1)
        }
    }
    static func execute() throws {
        let status = SecKeychainSetUserInteractionAllowed(false)
        guard status == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
        let args = Array(CommandLine.arguments.dropFirst())
        if args.count == 2 && ["csr", "test-csr"].contains(args[0]) {
            let label = "Apple Bridge Developer ID release signing"
            let tag = Data("dev.applebridge.release.developer-id".utf8)
            let query: [CFString: Any] = [kSecClass: kSecClassKey, kSecAttrApplicationTag: tag, kSecAttrKeyClass: kSecAttrKeyClassPrivate, kSecReturnRef: true]
            var found: CFTypeRef?
            let existing = SecItemCopyMatching(query as CFDictionary, &found)
            let key: SecKey
            if args[0] == "test-csr" {
                var error: Unmanaged<CFError>?
                guard let temporary = SecKeyCreateRandomKey([kSecAttrKeyType: kSecAttrKeyTypeRSA, kSecAttrKeySizeInBits: 2048] as CFDictionary, &error) else { throw error!.takeRetainedValue() }
                key = temporary
            } else if existing == errSecSuccess, let found {
                key = (found as! SecKey)
            } else {
                guard existing == errSecItemNotFound else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(existing)) }
                var trusted: SecTrustedApplication?
                let trustedStatus = SecTrustedApplicationCreateFromPath("/usr/bin/codesign", &trusted)
                guard trustedStatus == errSecSuccess, let trusted else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(trustedStatus)) }
                var access: SecAccess?
                let accessStatus = SecAccessCreate(label as CFString, [trusted] as CFArray, &access)
                guard accessStatus == errSecSuccess, let access else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(accessStatus)) }
                let attrs: [CFString: Any] = [kSecAttrKeyType: kSecAttrKeyTypeRSA, kSecAttrKeySizeInBits: 2048, kSecAttrAccess: access, kSecPrivateKeyAttrs: [kSecAttrIsPermanent: true, kSecAttrIsExtractable: false, kSecAttrApplicationTag: tag, kSecAttrLabel: label]]
                var error: Unmanaged<CFError>?
                guard let created = SecKeyCreateRandomKey(attrs as CFDictionary, &error) else { throw error!.takeRetainedValue() }
                key = created
            }
            var error: Unmanaged<CFError>?
            guard let publicKey = SecKeyCopyPublicKey(key), let exported = SecKeyCopyExternalRepresentation(publicKey, &error) as Data? else { throw ReleaseError.invalidSignature }
            let rsaAlgorithm = der(0x30, oid([0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]) + der(0x05, Data()))
            let subject = der(0x30, der(0x31, der(0x30, oid([0x55, 0x04, 0x03]) + der(0x0c, Data(label.utf8)))))
            let request = der(0x30, der(0x02, Data([0])) + subject + der(0x30, rsaAlgorithm + der(0x03, Data([0]) + exported)) + der(0xa0, Data()))
            guard let signature = SecKeyCreateSignature(key, .rsaSignatureMessagePKCS1v15SHA256, request as CFData, &error) as Data? else { throw error!.takeRetainedValue() }
            let algorithm = der(0x30, oid([0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b]) + der(0x05, Data()))
            let encoded = der(0x30, request + algorithm + der(0x03, Data([0]) + signature)).base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed])
            try ("-----BEGIN CERTIFICATE REQUEST-----\n" + encoded + "\n-----END CERTIFICATE REQUEST-----\n").write(toFile: args[1], atomically: true, encoding: .utf8)
            print("R1-IDENTITY-CSR passed=true")
        } else if args.count == 2 && args[0] == "import-certificate" {
            let data = try Data(contentsOf: URL(fileURLWithPath: args[1]))
            guard let certificate = SecCertificateCreateWithData(nil, data as CFData) else { throw ReleaseError.invalidSignature }
            let result = SecCertificateAddToKeychain(certificate, nil)
            guard result == errSecSuccess || result == errSecDuplicateItem else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(result)) }
            print("R1-IDENTITY-CERTIFICATE passed=true")
        } else {
            throw ReleaseError.invalidFeed
        }
    }
}
