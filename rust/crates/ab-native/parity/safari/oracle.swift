import Foundation
import AppKit
import SourceSupport
import SafariSource

func context(_ root: URL) throws -> JSON { try JSONDecoder().decode(JSON.self, from: Data(contentsOf: root.appendingPathComponent("context.json"))) }
func write(_ value: JSON) throws {
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf: encoder.encode(value) + Data([10]))
}
actor Sink: ObservationSink {
    let root: URL
    let coverage: JSON
    var ordinal = 0
    init(_ root: URL, coverage: JSON) { self.root = root; self.coverage = coverage }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        try write(.object(["kind": .string("source_state"), "source": .string(source), "state": .string(state), "coverage": coverage, "capabilities": .array(capabilities.map(JSON.string)), "reason": .string(reason)]))
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.unsupported }
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        if scope == ["safari.tab"] { return }
        let ctx = try context(root)
        let rows = observations.map { row -> JSON in var fields = row.object; fields["observed_at"] = ctx["observed_at"]; return .object(fields) }
        let suffix = "-" + String(ordinal)
        try write(.object(["source": .string(source), "generation": .string(generation), "checkpoint": .string(ctx["checkpoint"].string + suffix), "scan_id": .string(ctx["scan_id"].string + suffix), "scan_started_at": ctx["observed_at"], "scope": .array(scope.map(JSON.string)), "completion_evidence": .string(completionEvidence), "coverage": coverage, "observations": .array(rows), "live": .bool(live)]))
        ordinal += 1
    }
}
@main struct Oracle {
    static func main() async {
        do { try await run() } catch { exit(2) }
    }
    static func run() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let ctx = try context(root)
        let tabs = SafariTabs(execute: { _ in NSAppleEventDescriptor.list() })
        let source = SafariSource(directory: root, cacheURL: root.appendingPathComponent("swift-cache/cache.json"), tabs: tabs, clock: {
            let now = (try? context(root))?["now"].number ?? 0
            return Date(timeIntervalSince1970: now)
        })
        let readiness = await source.open()
        guard readiness.state == "ready" else { throw SourceFailure.unavailable }
        let sink = Sink(root, coverage: readiness.coverage)
        try write(.object(["kind": .string("ready"), "pid": .number(Double(ProcessInfo.processInfo.processIdentifier))]))
        let capture = Task { try await source.capture(.object(["generation": ctx["generation"]]), into: sink) }
        let commands = AsyncStream<String> { continuation in
            Task.detached { while let line = readLine() { continuation.yield(line) }; continuation.finish() }
        }
        for await _ in commands {}
        capture.cancel()
        _ = try? await capture.value
    }
}
