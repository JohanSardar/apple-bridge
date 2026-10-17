import Foundation

extension AppleTruth {
    static func usage(_ request: [String: Any]) throws -> [String: Any] {
        guard let root = request["snapshot_path"] as? String, let script = Bundle.main.resourceURL?.appendingPathComponent("usage.py") else { throw UsageTruthFailure.invalid }
        let process = Process()
        let output = URL(fileURLWithPath: root).appendingPathComponent("decoded.json")
        defer { try? FileManager.default.removeItem(at: output) }
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, root, TimeZone.current.identifier, output.path]
        process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0, let result = try JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any], result["status"] as? String == "verified" else { throw UsageTruthFailure.decode }
        return result
    }
}

private enum UsageTruthFailure: Error { case invalid, decode }
