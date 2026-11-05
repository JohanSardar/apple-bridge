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

    public static func primitiveResult(id: JSON, result: JSON) -> JSON {
        .object(["t": .string("PrimitiveResult"), "id": id, "result": result])
    }

    public static func primitiveError(id: JSON, code: String) -> JSON {
        .object(["t": .string("PrimitiveError"), "id": id, "code": .string(code)])
    }
}
