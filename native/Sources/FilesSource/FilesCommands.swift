import Foundation
import CryptoKit
import SourceSupport

extension FilesSource {
    static let commands = ["files.create", "files.write", "files.move", "files.rename", "files.trash", "files.set_tags", "files.download", "files.evict"]

    func confined(_ args: JSON, _ field: String, allowRoot: Bool = false) throws -> URL {
        let root = URL(fileURLWithPath: args["root"].string).standardizedFileURL
        guard roots.contains(root), !args[field].string.hasPrefix("/") else { throw SourceFailure.invalid }
        let parts = args[field].string.split(separator: "/", omittingEmptySubsequences: false)
        guard !parts.contains(".."), !parts.contains("."), !parts.contains("") || allowRoot && args[field].string.isEmpty else { throw SourceFailure.invalid }
        var url = root
        for part in parts where !part.isEmpty {
            url.appendPathComponent(String(part))
            if try url.resourceValues(forKeys: [.isSymbolicLinkKey]).isSymbolicLink == true { throw SourceFailure.invalid }
        }
        let resolved = url.standardizedFileURL.resolvingSymlinksInPath()
        guard resolved.path.hasPrefix(root.path + "/") || allowRoot && resolved == root else { throw SourceFailure.invalid }
        return resolved
    }

    func destination(_ args: JSON) throws -> URL {
        let parent = try confined(args, "parent", allowRoot: true)
        let name = args["name"].string
        guard !name.isEmpty, name != ".", name != "..", !name.contains("/"), !name.contains("\0") else { throw SourceFailure.invalid }
        let url = parent.appendingPathComponent(name)
        if FileManager.default.fileExists(atPath: url.path) {
            guard try url.resourceValues(forKeys: [.isSymbolicLinkKey]).isSymbolicLink != true else { throw SourceFailure.invalid }
        }
        return url
    }

    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"], args = action["args"], op = action["op"].string
        if op == "files.transform" { return Prepared(nativeState: .object(["route": .string("native")]), fingerprint: command["request_hash"].string, action: action) }
        guard Self.commands.contains(op) else { throw SourceFailure.unsupported }
        if op == "files.create" || op == "files.write" || op == "files.move" || op == "files.rename" { _ = try destination(args) }
        if op != "files.create" && op != "files.write" { _ = try confined(args, "path") }
        if op == "files.create" || op == "files.write" {
            guard let data = Data(base64Encoded: args["content_base64"].string), data.count <= 128 << 10,
                  SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == args["digest"].string else { throw SourceFailure.invalid }
        }
        let bytes = try JSONEncoder().encode(action)
        return Prepared(nativeState: .null, fingerprint: SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined(), action: action)
    }

    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        if prepared.action["op"].string == "files.transform" { return try transform(prepared.action["args"]) }
        _ = try await prepare(.object(["action": prepared.action]))
        let args = prepared.action["args"], op = prepared.action["op"].string
        let fm = FileManager.default
        var url: URL
        if op == "files.create" || op == "files.write" {
            url = try destination(args)
            guard let data = Data(base64Encoded: args["content_base64"].string) else { throw SourceFailure.invalid }
            if op == "files.create" { try data.write(to: url, options: .withoutOverwriting) }
            else { guard fm.fileExists(atPath: url.path) else { throw SourceFailure.conflict }; try data.write(to: url, options: .atomic) }
            guard try Data(contentsOf: url) == data else { throw SourceFailure.corrupt }
        } else {
            url = try confined(args, "path")
            switch op {
            case "files.move", "files.rename":
                let target = try destination(args)
                try fm.moveItem(at: url, to: target)
                guard !fm.fileExists(atPath: url.path), fm.fileExists(atPath: target.path) else { throw SourceFailure.corrupt }
                url = target
            case "files.trash":
                var trashed: NSURL?
                try fm.trashItem(at: url, resultingItemURL: &trashed)
                guard let result = trashed as URL?, fm.fileExists(atPath: result.path), !fm.fileExists(atPath: url.path), result.pathComponents.contains(".Trash") || result.pathComponents.contains(".Trashes") else { throw SourceFailure.corrupt }
                return .object(["observed": .bool(true), "trashed": .bool(true), "native_key": .string(url.path), "trash_path": .string(result.path)])
            case "files.set_tags":
                let tags = args["tags"].array.map { $0.string }
                var values = URLResourceValues()
                values.tagNames = tags
                try url.setResourceValues(values)
                url.removeAllCachedResourceValues()
                guard Set(try url.resourceValues(forKeys: [.tagNamesKey]).tagNames ?? []) == Set(tags) else { throw SourceFailure.corrupt }
            case "files.download", "files.evict":
                guard try url.resourceValues(forKeys: [.isUbiquitousItemKey]).isUbiquitousItem == true else { throw SourceFailure.unsupported }
                if op == "files.download" { try fm.startDownloadingUbiquitousItem(at: url) }
                else {
                    let uploadDeadline = Date().addingTimeInterval(50)
                    while true {
                        url.removeAllCachedResourceValues()
                        let values = try url.resourceValues(forKeys: [.ubiquitousItemIsUploadedKey, .ubiquitousItemUploadingErrorKey])
                        if values.ubiquitousItemIsUploaded == true { break }
                        guard values.ubiquitousItemUploadingError == nil, Date() < uploadDeadline else { throw SourceFailure.unavailable }
                        try await Task.sleep(for: .milliseconds(200))
                    }
                    try fm.evictUbiquitousItem(at: url)
                }
                let deadline = Date().addingTimeInterval(50)
                while true {
                    url.removeAllCachedResourceValues()
                    guard let status = try url.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]).ubiquitousItemDownloadingStatus else { throw SourceFailure.unavailable }
                    let downloaded = status != .notDownloaded
                    if downloaded == (op == "files.download") { break }
                    guard Date() < deadline else { throw SourceFailure.unavailable }
                    try await Task.sleep(for: .milliseconds(100))
                }
            default: throw SourceFailure.unsupported
            }
        }
        var result: [String: JSON] = ["observed": .bool(true), "path": .string(url.path), "native_key": .string(url.path)]
        if op == "files.create" || op == "files.write" {
            result["digest"] = .string(SHA256.hash(data: try Data(contentsOf: url)).map { String(format: "%02x", $0) }.joined())
        }
        if op == "files.set_tags" { result["tags"] = .array((try url.resourceValues(forKeys: [.tagNamesKey]).tagNames ?? []).map(JSON.string)) }
        if op == "files.download" || op == "files.evict" { result["downloaded"] = .bool(op == "files.download") }
        return .object(result)
    }
}
