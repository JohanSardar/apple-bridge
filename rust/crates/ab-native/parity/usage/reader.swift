import Foundation
import Decoders
import SourceSupport
import UsageSource

enum Finished: Error { case captured }
actor Sink: ObservationSink {
    let context: JSON
    init(_ context: JSON) { self.context = context }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw Finished.captured }
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        let rows = observations.map { row -> JSON in
            var object = row.object; object["observed_at"] = context["observed_at"]; return .object(object)
        }
        let coverage = await UsageSource.coverage
        let output: JSON = .object(["source": .string(source), "generation": context["generation"], "checkpoint": context["checkpoint"], "scan_id": context["scan_id"], "scan_started_at": context["observed_at"], "scope": .array(scope.map(JSON.string)), "completion_evidence": .string(completionEvidence), "coverage": coverage, "observations": .array(rows), "live": .bool(live)])
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        try FileHandle.standardOutput.write(contentsOf: encoder.encode(output) + Data([10]))
    }
}
@main
struct UsageFixtureOracle {
    @MainActor
    static func main() async {
        do { try await run() } catch { exit(2) }
    }
    @MainActor
    static func run() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let context = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: root.appendingPathComponent("context.json")))
        var blocked = context["blocked"].array.map { DateInterval(start: Date(timeIntervalSince1970: $0["start"].number), end: Date(timeIntervalSince1970: $0["end"].number)) }
        if context["history"] != .null {
            let history = context["history"]
            blocked = UsageHistory.blocked(path: root.appendingPathComponent(history["database"].string).path, identifier: history["hardware_id"].string, now: Date(timeIntervalSince1970: history["now"].number))
        }
        let intervals = blocked
        let source = UsageSource(root: root, blockedHistory: { intervals })
        if CommandLine.arguments.contains("--interactive") {
            var index = 0
            while let command = readLine() {
                guard command == "scan" else { throw Finished.captured }
                let current = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: root.appendingPathComponent("context.json")))
                try await source.scan(Sink(current), live: index > 0)
                index += 1
            }
        } else { try await source.scan(Sink(context), live: false) }
    }
}
