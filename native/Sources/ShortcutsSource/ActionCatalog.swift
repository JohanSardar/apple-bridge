import Foundation
import Darwin
import SourceSupport

public enum ActionCatalog {
    private static func read(_ url: URL, limit: Int) throws -> Data {
        let fd = Darwin.open(url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
        guard fd >= 0 else { throw SourceFailure.unavailable }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? handle.close() }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG, info.st_size <= Int64(limit) else { throw SourceFailure.invalid }
        let data = try handle.read(upToCount: limit + 1) ?? Data()
        guard data.count <= limit else { throw SourceFailure.invalid }
        return data
    }

    public static func discover(roots: [URL] = [URL(fileURLWithPath: "/Applications"), URL(fileURLWithPath: "/System/Applications"), FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications")]) throws -> JSON {
        var items: [JSON] = []
        var visited = 0
        var bytes = 0
        var complete = true
        var seen = Set<String>()
        var resultBytes = 0
        let deadline = Date().addingTimeInterval(10)
        for root in roots {
            guard FileManager.default.fileExists(atPath: root.path) else { continue }
            guard let tree = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isSymbolicLinkKey, .fileSizeKey], options: [.skipsHiddenFiles], errorHandler: { _, _ in complete = false; return false }) else { complete = false; continue }
            while let url = tree.nextObject() as? URL {
                try Task.checkCancellation()
                visited += 1
                if visited > 50000 || Date() > deadline || items.count >= 2000 { complete = false; break }
                let relative = url.path.dropFirst(root.path.count).split(separator: "/")
                if relative.count > 12 { tree.skipDescendants(); complete = false; continue }
                guard let values = try? url.resourceValues(forKeys: [.isSymbolicLinkKey, .fileSizeKey]), values.isSymbolicLink != true else { tree.skipDescendants(); continue }
                guard url.lastPathComponent == "extract.actionsdata", url.deletingLastPathComponent().lastPathComponent == "Metadata.appintents", relative.contains(where: { $0.hasSuffix(".app") }) else { continue }
                let size = values.fileSize ?? 0
                guard size > 0, size <= 4 << 20, bytes + size <= 32 << 20 else { complete = false; continue }
                bytes += size
                guard let data = try? read(url, limit: min(4 << 20, size)), data.count <= 4 << 20,
                      let document = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let actions = document["actions"] as? [String: Any] else { complete = false; continue }
                var bundle = url.deletingLastPathComponent()
                while !["app", "appex"].contains(bundle.pathExtension), bundle.path != "/" { bundle.deleteLastPathComponent() }
                guard let infoData = try? read(bundle.appendingPathComponent("Contents/Info.plist"), limit: min(1 << 20, (32 << 20) - bytes)), infoData.count <= 1 << 20, let info = try? PropertyListSerialization.propertyList(from: infoData, format: nil) as? [String: Any], let identifier = info["CFBundleIdentifier"] as? String, identifier.utf8.count <= 1024 else { complete = false; continue }
                bytes += infoData.count
                for key in actions.keys.sorted() {
                    try Task.checkCancellation()
                    if Date() > deadline { complete = false; break }
                    guard items.count < 2000 else { complete = false; break }
                    guard let action = actions[key] as? [String: Any] else { complete = false; continue }
                    if action["isDiscoverable"] as? Bool == false { continue }
                    if let visibility = action["visibilityMetadata"] as? [String: Any], visibility["isDiscoverable"] as? Bool == false || visibility["assistantOnly"] as? Bool == true { continue }
                    let id = action["identifier"] as? String ?? key
                    let title = (action["title"] as? [String: Any])?["key"] as? String ?? id
                    guard !id.isEmpty, id.utf8.count <= 1024, title.utf8.count <= 4096 else { complete = false; continue }
                    guard seen.insert(identifier + "/" + id).inserted else { continue }
                    let cost = (identifier.utf8.count + id.utf8.count + title.utf8.count) * 6 + 128
                    guard resultBytes + cost < 900000 else { complete = false; break }
                    resultBytes += cost
                    items.append(.object(["app": .string(identifier), "identifier": .string(id), "title": .string(title)]))
                }
            }
            if visited > 50000 || Date() > deadline || items.count >= 2000 { break }
        }
        return .object(["items": .array(items), "complete": .bool(complete), "schema_guaranteed": .bool(false)])
    }
}
