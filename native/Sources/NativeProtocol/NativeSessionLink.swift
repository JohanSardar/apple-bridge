import Foundation
import Darwin
import SourceSupport

public final class NativeSessionLink: @unchecked Sendable {
    public let fence: UInt64
    public let checkpoints: [String: JSON]
    private let wire: NativeWire
    private let descriptor: Int32

    public init(descriptor: Int32) throws {
        let wire = try NativeWire(descriptor: descriptor)
        let hello = try wire.receive()
        guard hello["t"].string == "Hello", hello["protocol"].string == "native/1", hello["version"] == .integer(1), hello["owner_fence"].unsigned > 0 else { throw SourceFailure.invalid }
        try wire.send(.object(["t": .string("Welcome"), "protocol": .string("native/1"), "version": .integer(1)]))
        let session = try wire.receive()
        guard session["t"].string == "Session" else { throw SourceFailure.invalid }
        self.wire = wire
        self.descriptor = descriptor
        fence = hello["owner_fence"].unsigned
        checkpoints = session["checkpoints"].object
    }

    public func receive() async throws -> JSON {
        let wire = self.wire
        return try await Task.detached { try wire.receive() }.value
    }

    public func send(_ frame: JSON) throws {
        try wire.send(frame)
    }

    public func close() {
        shutdown(descriptor, SHUT_RDWR)
    }
}

public enum NativeSessionFrame {
    public static func ready(coverage: [String: JSON], capabilities: [String: JSON], helperBuild: String, osBuild: String) -> JSON {
        .object(["t": .string("Ready"), "coverage": .object(coverage), "capabilities": .object(capabilities), "helper_build": .string(helperBuild), "os_build": .string(osBuild)])
    }

    public static func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) -> JSON {
        .object(["t": .string("SourceState"), "source": .string(source), "state": .string(state), "coverage": .object([source: coverage]), "capabilities": .object([source: .array(capabilities.map(JSON.string))]), "reason": .string(reason)])
    }

    public static func scan(id: String, source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) -> JSON {
        var frame: [String: JSON] = ["t": .string("Scan"), "id": .string(id), "source": .string(source), "generation": .string(generation), "observations": .array(observations), "live": .bool(live), "blob_handles": .object(blobHandles.mapValues(JSON.string))]
        if !scanID.isEmpty {
            frame["scan_id"] = .string(scanID)
            frame["scope"] = .array(scope.map(JSON.string))
            frame["completion_evidence"] = .string(completionEvidence)
            if !scanStartedAt.isEmpty { frame["scan_started_at"] = .string(scanStartedAt) }
        }
        return .object(frame)
    }

    public static func prepared(commandID: JSON, nativeState: JSON, route: String, candidateIDs: [JSON], fingerprint: String) -> JSON {
        .object(["t": .string("Prepared"), "command_id": commandID, "native_state": nativeState, "route": .string(route), "candidate_ids": .array(candidateIDs), "fingerprint": .string(fingerprint)])
    }

    public static func rejected(commandID: JSON, code: String, message: String, evidence: JSON?) -> JSON {
        var frame: [String: JSON] = ["t": .string("Rejected"), "command_id": commandID, "code": .string(code), "message": .string(message)]
        if let evidence, evidence != .null { frame["evidence"] = evidence }
        return .object(frame)
    }

    public static func outcome(commandID: JSON, disposition: String, evidence: JSON) -> JSON {
        .object(["t": .string("Outcome"), "command_id": commandID, "disposition": .string(disposition), "evidence": evidence])
    }

    public static func primitiveResult(id: JSON, result: JSON) -> JSON {
        .object(["t": .string("PrimitiveResult"), "id": id, "result": result])
    }

    public static func primitiveError(id: JSON, code: String) -> JSON {
        .object(["t": .string("PrimitiveError"), "id": id, "code": .string(code)])
    }
}
