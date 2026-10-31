import Foundation
import CryptoKit

struct ReleaseItem {
    let version: String
    let url: URL
    let length: Int
    let signature: Data
}

enum ReleaseError: LocalizedError {
    case invalidFeed, invalidVersion, invalidSignature, unsafePath, cleanupFailed
    var errorDescription: String? {
        switch self {
        case .invalidFeed: return "The update feed is invalid. Try checking for updates again later."
        case .invalidVersion: return "The update version is invalid. Download a fresh copy of Apple Bridge."
        case .invalidSignature: return "The update signature could not be verified. Download a fresh copy of Apple Bridge."
        case .unsafePath: return "The data folder contains a redirected path. Remove that link before uninstalling."
        case .cleanupFailed: return "Apple Bridge could not finish uninstalling. Quit the app and try again."
        }
    }
}

final class ReleaseFeed: NSObject, XMLParserDelegate {
    private var items: [ReleaseItem] = []
    private var invalid = false
    private var inItem = false
    private var enclosureCount = 0
    private var sparkleNamespace = ""
    static func parse(_ data: Data) throws -> [ReleaseItem] {
        guard data.count <= 1_048_576, data.range(of: Data("<!DOCTYPE".utf8)) == nil else { throw ReleaseError.invalidFeed }
        let feed = ReleaseFeed()
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = true
        parser.shouldReportNamespacePrefixes = true
        parser.shouldResolveExternalEntities = false
        parser.delegate = feed
        guard parser.parse(), !feed.invalid, !feed.items.isEmpty else { throw ReleaseError.invalidFeed }
        return feed.items
    }
    func parser(_ parser: XMLParser, didStartMappingPrefix prefix: String, toURI uri: String) {
        if prefix == "sparkle" { sparkleNamespace = uri }
    }
    func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName: String?, attributes: [String: String]) {
        if name == "item" {
            if inItem { invalid = true }
            inItem = true
            enclosureCount = 0
        }
        guard name == "enclosure" else { return }
        enclosureCount += 1
        guard inItem, enclosureCount == 1, sparkleNamespace == "http://www.andymatuschak.org/xml-namespaces/sparkle",
              let version = attributes["sparkle:version"], (try? ReleaseVersion.components(version)) != nil,
              let rawURL = attributes["url"], let url = URL(string: rawURL), url.scheme == "https", url.host != nil, url.user == nil, url.password == nil,
              let rawLength = attributes["length"], let length = Int(rawLength), length > 0,
              let rawSignature = attributes["sparkle:edSignature"], let signature = Data(base64Encoded: rawSignature), signature.count == 64 else {
            invalid = true
            return
        }
        items.append(ReleaseItem(version: version, url: url, length: length, signature: signature))
    }
    func parser(_ parser: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName: String?) {
        if name == "item" {
            if enclosureCount != 1 { invalid = true }
            inItem = false
        }
    }
}

enum ReleaseVersion {
    static func components(_ value: String) throws -> [UInt64] {
        let parts = value.split(separator: ".", omittingEmptySubsequences: false)
        guard !parts.isEmpty, parts.count <= 4 else { throw ReleaseError.invalidVersion }
        return try parts.map {
            guard !$0.isEmpty, $0.allSatisfy({ $0.isASCII && $0.isNumber }), let n = UInt64($0) else { throw ReleaseError.invalidVersion }
            return n
        }
    }
    static func newer(_ candidate: String, than installed: String) throws -> Bool {
        let a = try components(candidate)
        let b = try components(installed)
        for i in 0..<max(a.count, b.count) {
            let x = i < a.count ? a[i] : 0
            let y = i < b.count ? b[i] : 0
            if x != y { return x > y }
        }
        return false
    }
}

enum ReleaseSignature {
    static func verify(_ data: Data, item: ReleaseItem, publicKey: Data) throws {
        guard data.count == item.length,
              let key = try? Curve25519.Signing.PublicKey(rawRepresentation: publicKey),
              key.isValidSignature(item.signature, for: data) else { throw ReleaseError.invalidSignature }
    }
}

enum ReleaseCleanup {
    static func paths(home: URL) -> [URL] {
        ["Library/Application Support/AppleBridge", "Library/Caches/dev.applebridge.helper", "Library/Preferences/dev.applebridge.helper.plist", "Library/Saved Application State/dev.applebridge.helper.savedState"].map { home.appendingPathComponent($0) }
    }
    static func run(home: URL, unregister: () throws -> Void, removeCredentials: () throws -> Void, resetPermissions: () throws -> Void) throws {
        let manager = FileManager.default
        let root = home.standardizedFileURL
        guard root.path != "/", root.resolvingSymlinksInPath().path == root.path else { throw ReleaseError.unsafePath }
        for path in paths(home: root) {
            var parent = path.deletingLastPathComponent()
            while parent.path != root.path {
                guard parent.path.hasPrefix(root.path + "/"), parent.resolvingSymlinksInPath().path == parent.path else { throw ReleaseError.unsafePath }
                parent.deleteLastPathComponent()
            }
        }
        try unregister()
        try removeCredentials()
        try resetPermissions()
        for path in paths(home: root) where manager.fileExists(atPath: path.path) || (try? manager.destinationOfSymbolicLink(atPath: path.path)) != nil {
            try manager.removeItem(at: path)
        }
    }
}
