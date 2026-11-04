import Foundation

public enum MessagesMediaFact {
    public static func answer(_ args: [String: Any], allowedRoot: URL, paused: @Sendable () -> Bool = { MediaDerivatives.paused }) async throws -> [String: Any] {
        guard Set(args.keys).isSubset(of: ["digest", "file", "uti", "mime", "sticker", "companions"]),
              let digest = args["digest"] as? String, MediaDerivatives.validDigest(digest),
              let file = args["file"] as? String, file.hasPrefix("/"),
              args["uti"] == nil || args["uti"] is String, args["mime"] == nil || args["mime"] is String,
              args["companions"] == nil || args["companions"] is [String],
              args["sticker"] == nil || args["sticker"] is Bool else { throw MediaFailure.invalid }
        let uti = args["uti"] as? String ?? ""
        let mime = args["mime"] as? String ?? ""
        let companions = args["companions"] as? [String] ?? []
        guard companions.count <= 16 else { throw MediaFailure.invalid }
        let root = allowedRoot.resolvingSymlinksInPath().path + "/"
        func confined(_ path: String) -> URL? {
            guard path.hasPrefix("/") else { return nil }
            let url = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath()
            return url.path.hasPrefix(root) ? url : nil
        }
        guard let input = confined(file), companions.allSatisfy({ confined($0) != nil }) else { throw MediaFailure.invalid }
        var answer: [String: Any] = ["digest": digest, "root": MediaDerivatives.root.path]
        guard MediaDerivatives.supports(input, uti: uti, mime: mime) else {
            answer["status"] = "unsupported"
            return answer
        }
        if paused() {
            answer["status"] = "paused"
            return answer
        }
        let usable = companions.compactMap(confined).filter { FileManager.default.fileExists(atPath: $0.path) && MediaDerivatives.posterSourceAvailable($0) }
        let sticker = args["sticker"] as? Bool ?? false
        do {
            let result = try await Task.detached(executorPreference: MediaExecutor.shared, priority: .utility) {
                try await MediaDerivatives.make(input, digest: digest, companions: usable, sticker: sticker)
            }.value
            answer["status"] = "ready"
            answer["result"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(result))
        } catch {
            answer["status"] = "failed"
        }
        return answer
    }
}
