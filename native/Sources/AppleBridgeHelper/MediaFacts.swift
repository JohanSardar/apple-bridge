import Foundation
import UniformTypeIdentifiers
import MediaDerivatives

enum MediaFacts {
    static func run() -> Int32 {
        guard let input = try? FileHandle.standardInput.readToEnd(),
              let request = try? JSONSerialization.jsonObject(with: input) as? [String: Any],
              let kind = request["kind"] as? String,
              let response = answer(kind, request),
              let output = try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]) else { return 1 }
        FileHandle.standardOutput.write(output)
        return 0
    }

    private static func answer(_ kind: String, _ request: [String: Any]) -> [String: Any]? {
        switch kind {
        case "mime":
            guard let uti = request["uti"] as? String else { return nil }
            var response: [String: Any] = [:]
            if let mime = UTType(uti)?.preferredMIMEType { response["mime"] = mime }
            return response
        case "supports":
            guard let uti = request["uti"] as? String, let ext = request["ext"] as? String else { return nil }
            return ["supports": MediaDerivatives.supports(URL(fileURLWithPath: "/x." + ext), uti: uti)]
        case "derive":
            guard let digest = request["digest"] as? String, let file = request["file"] as? String else { return nil }
            return derive(digest: digest, file: file)
        default:
            return nil
        }
    }

    private static func derive(digest: String, file: String) -> [String: Any] {
        let semaphore = DispatchSemaphore(value: 0)
        let box = Box()
        Task.detached {
            box.value = try? await MediaDerivatives.make(URL(fileURLWithPath: file), digest: digest)
            semaphore.signal()
        }
        semaphore.wait()
        guard let result = box.value else { return ["status": "unavailable"] }
        var variants: [String: Any] = [:]
        for (name, variant) in result.derivatives {
            var entry: [String: Any] = ["path": MediaDerivatives.url(variant).path, "mime": variant.mime]
            if let width = variant.width { entry["width"] = width }
            if let height = variant.height { entry["height"] = height }
            variants[name] = entry
        }
        return ["status": result.unavailable?.isEmpty == false ? "partial" : "complete", "variants": variants]
    }

    private final class Box: @unchecked Sendable {
        var value: MediaResult?
    }
}
