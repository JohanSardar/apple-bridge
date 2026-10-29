import Foundation
import SourceSupport

struct SafariCaptureCache: Codable, Sendable {
    struct Key: Codable, Sendable {
        let collection: String
        let nativeKey: String
    }
    struct Scan: Codable, Sendable {
        let generation: String
        let keys: [Key]
    }
    let generation: String
    let scans: [String: Scan]

    var rows: [String: [JSON]] {
        scans.mapValues { scan in scan.keys.map { .object(["collection": .string($0.collection), "native_key": .string($0.nativeKey), "data": .null]) } }
    }
    static func load(_ url: URL?) throws -> SafariCaptureCache? {
        guard let url, FileManager.default.fileExists(atPath: url.path) else { return nil }
        let cache = try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
        guard !cache.generation.isEmpty else { throw SourceFailure.corrupt }
        let scopes = ["history": ["safari.history"], "bookmarks": ["safari.bookmark", "safari.reading_list"], "cloud": ["safari.cloud_tab"]]
        for (kind, scan) in cache.scans {
            guard let scope = scopes[kind], !scan.generation.isEmpty,
                  scan.keys.allSatisfy({ scope.contains($0.collection) && !$0.nativeKey.isEmpty }),
                  Set(scan.keys.map { $0.collection + "/" + $0.nativeKey }).count == scan.keys.count else { throw SourceFailure.corrupt }
        }
        return cache
    }
    static func save(generation: String, databaseGenerations: [String: String], rows: [String: [JSON]], to url: URL?) throws {
        guard let url else { return }
        let scans = databaseGenerations.map { kind, generation in
            (kind, Scan(generation: generation, keys: (rows[kind] ?? []).map { Key(collection: $0["collection"].string, nativeKey: $0["native_key"].string) }))
        }
        let cache = Self(generation: generation, scans: Dictionary(uniqueKeysWithValues: scans))
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try JSONEncoder().encode(cache).write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
}
