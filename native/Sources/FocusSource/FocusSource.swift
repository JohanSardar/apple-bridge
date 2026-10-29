import Foundation
import Intents
import SourceSupport
import ShortcutsSource

@MainActor
public final class FocusSource: Source {
    public nonisolated let name = "focus"
    private let generation = UUID().uuidString
    private let readStatus: @MainActor () -> JSON
    private let run: @MainActor (JSON) async throws -> JSON
    public init(status: (@MainActor () -> JSON)? = nil, runner: (@MainActor (JSON) async throws -> JSON)? = nil) {
        readStatus = status ?? Self.status
        run = runner ?? { request in
            let source = ShortcutsSource()
            var normalized = request.object
            normalized["until"] = .string(request["until"].string)
            let input = try JSONEncoder().encode(JSON.object(normalized))
            let prepared = try await source.prepare(.object(["request_hash": .string(UUID().uuidString), "action": .object(["op": .string("shortcuts.run"), "args": .object(["shortcut": .string("AB Focus"), "input": .string(String(decoding: input, as: UTF8.self)), "timeout_ms": .number(15000)])])]))
            return try await source.dispatch(prepared)
        }
    }
    private static func expiry(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: value) { return date }
        formatter.formatOptions.insert(.withFractionalSeconds)
        return formatter.date(from: value)
    }
    public static func status() -> JSON {
        let center = INFocusStatusCenter.default
        let authorization: String
        switch center.authorizationStatus {
        case .authorized: authorization = "authorized"
        case .denied: authorization = "denied"
        case .restricted: authorization = "restricted"
        default: authorization = "not_determined"
        }
        let focused: JSON = center.authorizationStatus == .authorized ? center.focusStatus.isFocused.map { .bool($0) } ?? .null : .null
        return .object(["focused": focused, "name": .null, "authorization": .string(authorization)])
    }
    private var coverage: JSON { .object(["fields": .string("partial"), "missing": .array([.string("named Focus identity and restoration unavailable")])]) }
    public func open() async -> SourceReadiness {
        let authorized = readStatus()["authorization"].string == "authorized"
        return SourceReadiness(state: "ready", coverage: coverage, capabilities: authorized ? ["focus.set"] : [], reason: authorized ? "" : "needs Focus status permission")
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        var previous: JSON?
        while !Task.isCancelled {
            let data = readStatus()
            if data != previous {
                let started = timestamp()
                let observation: JSON = .object(["kind": .string("upsert"), "collection": .string("focus.status"), "native_key": .string("current"), "scoped_alias": .string("focus/status/current"), "observed_at": .string(started), "data": data])
                try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: [observation], scanID: UUID().uuidString, scanStartedAt: started, scope: ["focus.status"], completionEvidence: "complete", blobHandles: [:], live: false)
                previous = data
            }
            try await Task.sleep(for: .seconds(30))
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        let args = action["args"]
        guard action["op"].string == "focus.set", args["mode"].string == "Do Not Disturb", args["on"] == .bool(true) || args["on"] == .bool(false) else { throw SourceFailure.invalid }
        if !args["until"].string.isEmpty {
            guard args["on"].bool, let until = Self.expiry(args["until"].string), until > Date() else { throw SourceFailure.invalid }
        }
        guard readStatus()["authorization"].string == "authorized", readStatus()["focused"] != .null else { throw SourceFailure.permissionMissing }
        return Prepared(nativeState: .null, fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        guard readStatus()["authorization"].string == "authorized", readStatus()["focused"] != .null else { throw SourceFailure.permissionMissing }
        let args = prepared.action["args"]
        if !args["until"].string.isEmpty {
            guard let until = Self.expiry(args["until"].string), until > Date() else { throw SourceFailure.invalid }
        }
        let result = try await run(args)
        guard result["exit_status"] == .number(0), !result["timed_out"].bool else { throw SourceFailure.unavailable }
        for _ in 0..<20 {
            let status = readStatus()
            if status["authorization"].string == "authorized", status["focused"] == args["on"] {
                return .object(["disposition": .string("locally_observed"), "observed": .bool(true), "fingerprint": .string(prepared.fingerprint), "readback": status])
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        throw SourceFailure.unavailable
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard evidence["prepared"]["action"]["op"].string == "focus.set", evidence["fingerprint"] == evidence["prepared"]["request_hash"], evidence["observed"].bool, evidence["readback"]["authorization"].string == "authorized", evidence["readback"]["focused"] == evidence["prepared"]["action"]["args"]["on"] else { return .null }
        return evidence
    }
}
