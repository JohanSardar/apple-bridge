import Foundation
import Darwin
import SourceSupport
import ShortcutsSource

actor Sink: ObservationSink {
    var values: [JSON] = []
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard scope == ["shortcuts.folder", "shortcuts.shortcut"], completionEvidence == "complete", !generation.isEmpty else { throw SourceFailure.invalid }
        values = observations
    }
}

@main
struct Checks {
    static func main() async throws {
        let source = ShortcutsSource(executable: URL(fileURLWithPath: CommandLine.arguments[1]))
        if CommandLine.arguments[2] == "SC-CAPTURE" {
            let readiness = await source.open()
            guard readiness.coverage["pack_version"].string.isEmpty else { throw SourceFailure.corrupt }
            let marker = URL(fileURLWithPath: CommandLine.arguments[1]).deletingLastPathComponent().appendingPathComponent("pack")
            try Data().write(to: marker)
            let installed = await source.open()
            guard installed.coverage["pack_version"].string == "1" else { throw SourceFailure.corrupt }
            try FileManager.default.removeItem(at: marker)
            let sink = Sink()
            let task = Task { try await source.capture(.null, into: sink) }
            defer { task.cancel() }
            for _ in 0..<100 {
                let values = await sink.values
                if values.count == 2 {
                    guard values[0]["collection"].string == "shortcuts.folder", values[1]["data"]["name"].string == "Synthetic", values[1]["data"]["folder_id"]["$ref"]["native_key"].string == "00000000-0000-0000-0000-000000000001" else { throw SourceFailure.corrupt }
                    return
                }
                try await Task.sleep(for: .milliseconds(50))
            }
            throw SourceFailure.unavailable
        }
        let blob = try await source.prepare(.object(["request_hash": .string("synthetic"), "action": .object(["op": .string("shortcuts.run"), "args": .object(["shortcut": .string("echo"), "input_base64": .string("AAH/")])])]))
        let blobResult = try await source.dispatch(blob)
        guard blobResult["output_base64"].string == "AAH/", blobResult["output_bytes"].number == 3, blobResult["output_sha256"].string.count == 64 else { throw SourceFailure.corrupt }
        let flood = try await source.prepare(.object(["request_hash": .string("synthetic"), "action": .object(["op": .string("shortcuts.run"), "args": .object(["shortcut": .string("flood"), "timeout_ms": .number(10000)])])]))
        let started = Date()
        do {
            _ = try await source.dispatch(flood)
            throw SourceFailure.corrupt
        } catch SourceFailure.unavailable {
            guard Date().timeIntervalSince(started) < 5 else { throw SourceFailure.corrupt }
        }
        let slow = try await source.prepare(.object(["request_hash": .string("synthetic"), "action": .object(["op": .string("shortcuts.run"), "args": .object(["shortcut": .string("slow"), "input": .string("synthetic"), "timeout_ms": .number(20000)])])]))
        let slowStarted = Date()
        var completed = try await source.dispatch(slow).object
        completed["prepared"] = .object(["request_hash": .string("synthetic"), "action": slow.action])
        let retained = await source.resolve("synthetic", evidence: .object(completed))
        guard Date().timeIntervalSince(slowStarted) > 15, retained["observed"].bool, retained["disposition"].string == "locally_observed", retained["output_base64"].string == Data("synthetic".utf8).base64EncodedString() else { throw SourceFailure.corrupt }
        completed["exit_status"] = .number(7)
        guard await source.resolve("synthetic", evidence: .object(completed)) == .null else { throw SourceFailure.corrupt }
        for (name, expected) in [("echo", 0), ("fail", 7), ("timeout", -1), ("binary", 0)] {
            let prepared = try await source.prepare(.object(["request_hash": .string("synthetic"), "action": .object(["op": .string("shortcuts.run"), "args": .object(["shortcut": .string(name), "input": .string("[ab-test synthetic]"), "timeout_ms": .number(name == "timeout" ? 100 : 30000)])])]))
            let result = try await source.dispatch(prepared)
            guard result["exit_status"].number == Double(expected), result["timed_out"].bool == (name == "timeout") else { throw NSError(domain: "fixture_" + name + "_status_" + String(result["exit_status"].number) + "_timeout_" + String(result["timed_out"].bool), code: 1) }
            if name == "timeout" {
                let path = URL(fileURLWithPath: CommandLine.arguments[1]).deletingLastPathComponent().appendingPathComponent("child")
                let child = Int32(try String(contentsOf: path, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines))!
                try await Task.sleep(for: .milliseconds(100))
                guard kill(child, 0) == -1, errno == ESRCH else { throw SourceFailure.corrupt }
            }
            if name == "echo", result["output_base64"].string != Data("[ab-test synthetic]".utf8).base64EncodedString() { throw SourceFailure.corrupt }
            if name == "binary", result["output_base64"].string != Data([0, 1, 255]).base64EncodedString() { throw SourceFailure.corrupt }
        }
    }
}
