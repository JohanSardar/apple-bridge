import CryptoKit
import Darwin
import Foundation
import Security

public enum CodeIdentity {
    public enum Failure: Error, Equatable {
        case unsigned
        case rejected
    }

    public static func verifyFile(_ path: String, identifier: String) throws {
        let requirement = try requirement(identifier: identifier)
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, [], &code) == errSecSuccess, let code else { throw Failure.rejected }
        let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate)
        guard SecStaticCodeCheckValidity(code, flags, requirement) == errSecSuccess else { throw Failure.rejected }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              let values = information as? [String: Any],
              let signatureFlags = values[kSecCodeInfoFlags as String] as? UInt32,
              signatureFlags & hardenedRuntime != 0 else { throw Failure.rejected }
    }

    public static func verifyProcess(pid: pid_t, identifier: String) throws {
        try verifyGuest([kSecGuestAttributePid: pid], identifier: identifier)
    }

    public static func verifySocketPeer(_ descriptor: Int32, identifier: String) throws {
        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        guard getsockopt(descriptor, SOL_LOCAL, LOCAL_PEERTOKEN, &token, &length) == 0, length == MemoryLayout<audit_token_t>.size else { throw Failure.rejected }
        let data = withUnsafeBytes(of: &token) { Data($0) }
        try verifyGuest([kSecGuestAttributeAudit: data], identifier: identifier)
    }

    private static let hardenedRuntime: UInt32 = 0x10000

    private static func verifyGuest(_ attributes: [CFString: Any], identifier: String) throws {
        let requirement = try requirement(identifier: identifier)
        var code: SecCode?
        guard SecCodeCopyGuestWithAttributes(nil, attributes as CFDictionary, [], &code) == errSecSuccess, let code else { throw Failure.rejected }
        guard SecCodeCheckValidity(code, [], requirement) == errSecSuccess else { throw Failure.rejected }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(unsafeBitCast(code, to: SecStaticCode.self), SecCSFlags(rawValue: kSecCSDynamicInformation), &information) == errSecSuccess,
              let values = information as? [String: Any],
              let status = values[kSecCodeInfoStatus as String] as? UInt32,
              status & hardenedRuntime != 0 else { throw Failure.rejected }
    }

    private static func requirement(identifier: String) throws -> SecRequirement {
        var requirement: SecRequirement?
        let text = "identifier \"\(identifier)\" and certificate leaf = H\"\(try ownLeafHash())\""
        guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess, let requirement else { throw Failure.rejected }
        return requirement
    }

    private static func ownLeafHash() throws -> String {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { throw Failure.unsigned }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { throw Failure.unsigned }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              let values = information as? [String: Any],
              let certificates = values[kSecCodeInfoCertificates as String] as? [SecCertificate],
              let leaf = certificates.first else { throw Failure.unsigned }
        return Insecure.SHA1.hash(data: SecCertificateCopyData(leaf) as Data).map { String(format: "%02x", $0) }.joined()
    }
}
