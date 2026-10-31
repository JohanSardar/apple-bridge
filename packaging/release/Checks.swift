import Foundation
import CryptoKit

@main
struct ReleaseChecks {
    static func main() throws {
        let key = Curve25519.Signing.PrivateKey()
        let payload = Data("[ab-test] update".utf8)
        let signature = try key.signature(for: payload)
        func feed(_ attrs: String) -> Data {
            Data("<rss xmlns:sparkle=\"http://www.andymatuschak.org/xml-namespaces/sparkle\"><channel><item><enclosure \(attrs)/></item></channel></rss>".utf8)
        }
        let attributes = "sparkle:version=\"2\" url=\"https://example.invalid/update.zip\" length=\"\(payload.count)\" sparkle:edSignature=\"\(signature.base64EncodedString())\""
        let item = try ReleaseFeed.parse(feed(attributes))[0]
        func refuses(_ action: () throws -> Void) -> Bool {
            do { try action(); return false } catch { return true }
        }
        precondition(item.version == "2" && item.length == payload.count)
        precondition(refuses { _ = try ReleaseFeed.parse(feed(attributes.replacingOccurrences(of: "https://", with: "http://"))) })
        precondition(refuses { _ = try ReleaseFeed.parse(feed("url=\"https://example.invalid\"")) })
        precondition(refuses { _ = try ReleaseFeed.parse(Data("<rss>".utf8)) })
        precondition(refuses { _ = try ReleaseFeed.parse(feed(attributes.replacingOccurrences(of: "length=\"\(payload.count)\"", with: "length=\"0\""))) })
        precondition(refuses { _ = try ReleaseFeed.parse(Data(String(decoding: feed(attributes), as: UTF8.self).replacingOccurrences(of: "http://www.andymatuschak.org/xml-namespaces/sparkle", with: "https://example.invalid/namespace").utf8)) })
        print("R1-APPCAST passed=true")
        let newer = try ReleaseVersion.newer("1.10", than: "1.9")
        precondition(newer)
        let equivalent = try ReleaseVersion.newer("1.0.0", than: "1")
        precondition(!equivalent)
        let older = try ReleaseVersion.newer("1", than: "2")
        precondition(!older)
        precondition(refuses { _ = try ReleaseVersion.components("1.beta") })
        precondition(refuses { _ = try ReleaseVersion.components("1..2") })
        precondition(refuses { _ = try ReleaseVersion.components("18446744073709551616") })
        print("R1-VERSION passed=true")
        try ReleaseSignature.verify(payload, item: item, publicKey: key.publicKey.rawRepresentation)
        precondition(refuses { try ReleaseSignature.verify(Data("tampered".utf8), item: item, publicKey: key.publicKey.rawRepresentation) })
        precondition(refuses { try ReleaseSignature.verify(payload, item: item, publicKey: Curve25519.Signing.PrivateKey().publicKey.rawRepresentation) })
        var tampered = payload
        tampered[0] ^= 1
        precondition(refuses { try ReleaseSignature.verify(tampered, item: item, publicKey: key.publicKey.rawRepresentation) })
        print("R1-SIGNATURE passed=true")
        let fm = FileManager.default
        let temp = fm.temporaryDirectory.appendingPathComponent("ab-test-release-\(UUID().uuidString)")
        try fm.createDirectory(at: temp, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: temp) }
        let home = temp.resolvingSymlinksInPath()
        let keep = home.appendingPathComponent("Library/Application Support/OtherApp/keep")
        try fm.createDirectory(at: keep.deletingLastPathComponent(), withIntermediateDirectories: true)
        try payload.write(to: keep)
        for path in ReleaseCleanup.paths(home: home) {
            try fm.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
            try payload.write(to: path)
        }
        precondition(refuses { try ReleaseCleanup.run(home: home, unregister: { throw ReleaseError.cleanupFailed }, removeCredentials: {}, resetPermissions: {}) })
        precondition(fm.fileExists(atPath: ReleaseCleanup.paths(home: home)[0].path))
        var order: [Int] = []
        try ReleaseCleanup.run(home: home, unregister: { order.append(1) }, removeCredentials: { order.append(2) }, resetPermissions: { order.append(3) })
        precondition(order == [1, 2, 3])
        precondition(ReleaseCleanup.paths(home: home).allSatisfy { !fm.fileExists(atPath: $0.path) })
        precondition(fm.fileExists(atPath: keep.path))
        let redirected = home.appendingPathComponent("Library/Caches")
        try fm.removeItem(at: redirected)
        try fm.createSymbolicLink(at: redirected, withDestinationURL: keep.deletingLastPathComponent())
        precondition(refuses { try ReleaseCleanup.run(home: home, unregister: {}, removeCredentials: {}, resetPermissions: {}) })
        precondition(fm.fileExists(atPath: keep.path))
        print("R1-UNINSTALL passed=true")
    }
}
