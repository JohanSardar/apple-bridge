import CryptoKit
import Foundation
import Security

struct PairCode: Decodable {
    let id: String
    let purpose: String
    let address: String
    let fingerprint: String
    let secret: Data
    let expires_at: String
    let remote: Bool?

    static func decode(_ code: String) throws -> PairCode {
        guard code.hasPrefix("ab1-") else { throw IdentityError.invalidCode }
        let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567")
        var buffer: UInt64 = 0
        var bits = 0
        var data = Data()
        for c in code.dropFirst(4) {
            guard let value = alphabet.firstIndex(of: c) else { throw IdentityError.invalidCode }
            buffer = (buffer << 5) | UInt64(value)
            bits += 5
            if bits >= 8 { bits -= 8; data.append(UInt8((buffer >> bits) & 255)) }
        }
        let result = try JSONDecoder().decode(PairCode.self, from: data)
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let expiry = formatter.date(from: result.expires_at) ?? ISO8601DateFormatter().date(from: result.expires_at)
        guard let url = URL(string: result.address), url.scheme == "https", url.host != nil, url.user == nil, url.query == nil, url.fragment == nil, url.path.isEmpty || url.path == "/", result.purpose == "iphone", result.secret.count == 20, result.fingerprint.count == 64, let expiry, expiry > Date() else { throw IdentityError.invalidCode }
        return result
    }
}

final class GatewayTLS: NSObject, URLSessionDelegate, @unchecked Sendable {
    let fingerprint: String
    let remote: Bool

    init(fingerprint: String, remote: Bool) {
        self.fingerprint = fingerprint
        self.remote = remote
    }

    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge, completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        guard !remote, challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust, let trust = challenge.protectionSpace.serverTrust, let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate], let certificate = chain.first else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        let digest = SHA256.hash(data: SecCertificateCopyData(certificate) as Data).map { String(format: "%02x", $0) }.joined()
        if digest == fingerprint {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            completionHandler(.cancelAuthenticationChallenge, nil)
        }
    }
}

struct PairingClient {
    static func request(address: String, fingerprint: String, remote: Bool, path: String, body: Data) async throws -> Data {
        guard let url = URL(string: address + path) else { throw IdentityError.invalidCode }
        let session = URLSession(configuration: .ephemeral, delegate: GatewayTLS(fingerprint: fingerprint, remote: remote), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = body
        request.timeoutInterval = 15
        let (data, response) = try await session.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw IdentityError.requestFailed }
        return data
    }

    static func pair(_ raw: String) async throws -> DeviceIdentity {
        let code = try PairCode.decode(raw.trimmingCharacters(in: .whitespacesAndNewlines))
        var identity = try DeviceIdentity.load()
        let signing = try Curve25519.Signing.PrivateKey(rawRepresentation: identity.signing)
        let encryption = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: identity.encryption)
        let message = Data(("apple-bridge-iphone-pair/1\0" + code.id + "\0").utf8) + signing.publicKey.rawRepresentation + encryption.publicKey.rawRepresentation
        let key = SymmetricKey(data: Data(SHA256.hash(data: code.secret)))
        let body = try JSONSerialization.data(withJSONObject: ["id": code.id, "signing_key": signing.publicKey.rawRepresentation.base64EncodedString(), "encryption_key": encryption.publicKey.rawRepresentation.base64EncodedString(), "proof": Data(HMAC<SHA256>.authenticationCode(for: message, using: key)).base64EncodedString(), "signature": try signing.signature(for: message).base64EncodedString()])
        let response = try await request(address: code.address.trimmingCharacters(in: CharacterSet(charactersIn: "/")), fingerprint: code.fingerprint, remote: code.remote ?? false, path: "/v1/iphone/pair", body: body)
        let result = try JSONDecoder().decode(PairedDevice.self, from: response)
        identity.deviceID = result.id
        identity.gatewayKey = result.gateway_key
        identity.address = code.address.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        identity.fingerprint = code.fingerprint
        identity.remote = code.remote ?? false
        try identity.save()
        return identity
    }

    static func signedRequest(identity: DeviceIdentity, path: String, payload: Data = Data()) async throws -> Data {
        guard let id = identity.deviceID, let address = identity.address, let fingerprint = identity.fingerprint else { throw IdentityError.invalidCode }
        let at = Int64(Date().timeIntervalSince1970)
        let message = Data(("apple-bridge-iphone/1\0" + path + "\0" + String(at) + "\0").utf8) + payload
        let signature = try Curve25519.Signing.PrivateKey(rawRepresentation: identity.signing).signature(for: message)
        let body = try JSONSerialization.data(withJSONObject: ["device_id": id, "at": at, "payload": payload.base64EncodedString(), "signature": signature.base64EncodedString()])
        return try await request(address: address, fingerprint: fingerprint, remote: identity.remote ?? false, path: path, body: body)
    }
}

struct PairedDevice: Decodable { let id: String; let gateway_key: Data? }
