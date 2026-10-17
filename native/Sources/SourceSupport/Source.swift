import Foundation

public enum JSON: Codable, Sendable, Equatable {
    case object([String: JSON]), array([JSON]), string(String), integer(UInt64), number(Double), bool(Bool), null

    public static func == (left: JSON, right: JSON) -> Bool {
        switch (left, right) {
        case (.object(let a), .object(let b)): return a == b
        case (.array(let a), .array(let b)): return a == b
        case (.string(let a), .string(let b)): return a == b
        case (.integer(let a), .integer(let b)): return a == b
        case (.number(let a), .number(let b)): return a == b
        case (.integer(let a), .number(let b)), (.number(let b), .integer(let a)): return UInt64(exactly: b) == a
        case (.bool(let a), .bool(let b)): return a == b
        case (.null, .null): return true
        default: return false
        }
    }

    public init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let result = try? value.decode(Bool.self) { self = .bool(result) }
        else if let result = try? value.decode(String.self) { self = .string(result) }
        else if let result = try? value.decode(UInt64.self) { self = .integer(result) }
        else if let result = try? value.decode(Double.self) { self = .number(result) }
        else if let result = try? value.decode([String: JSON].self) { self = .object(result) }
        else { self = .array(try value.decode([JSON].self)) }
    }

    public func encode(to encoder: any Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .object(let result): try value.encode(result)
        case .array(let result): try value.encode(result)
        case .string(let result): try value.encode(result)
        case .integer(let result): try value.encode(result)
        case .number(let result): try value.encode(result)
        case .bool(let result): try value.encode(result)
        case .null: try value.encodeNil()
        }
    }

    public subscript(_ name: String) -> JSON { if case .object(let value) = self { return value[name] ?? .null }; return .null }
    public var string: String { if case .string(let value) = self { return value }; return "" }
    public var object: [String: JSON] { if case .object(let value) = self { return value }; return [:] }
    public var array: [JSON] { if case .array(let value) = self { return value }; return [] }
    public var number: Double { if case .number(let value) = self { return value }; if case .integer(let value) = self { return Double(value) }; return 0 }
    public var unsigned: UInt64 { if case .integer(let value) = self { return value }; if case .number(let value) = self, value >= 0, value < Double(UInt64.max), value.rounded() == value { return UInt64(value) }; return 0 }
    public var bool: Bool { if case .bool(let value) = self { return value }; return false }
}

public enum SourceFailure: Error {
    case permissionMissing, unavailable, invalid, conflict, unsupported, expired, corrupt
}

public func timestamp(_ date: Date = Date()) -> String {
    date.ISO8601Format(.init(includingFractionalSeconds: true))
}

public func parseTimestamp(_ value: String) -> Date? {
    let format = ISO8601DateFormatter()
    format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return format.date(from: value) ?? ISO8601DateFormatter().date(from: value)
}

public struct SourceReadiness: Sendable {
    public let state: String
    public let coverage: JSON
    public let capabilities: [String]
    public let reason: String
    public init(state: String, coverage: JSON, capabilities: [String] = [], reason: String = "") {
        self.state = state; self.coverage = coverage; self.capabilities = capabilities; self.reason = reason
    }
}

public struct Prepared: Sendable {
    public let nativeState: JSON
    public let fingerprint: String
    public let action: JSON
    public init(nativeState: JSON, fingerprint: String, action: JSON) {
        self.nativeState = nativeState; self.fingerprint = fingerprint; self.action = action
    }
}

public protocol ObservationSink: Sendable {
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws
}

public protocol Source: Sendable {
    var name: String { get }
    func open() async -> SourceReadiness
    func capture(_ resume: JSON, into sink: any ObservationSink) async throws
    func prepare(_ command: JSON) async throws -> Prepared
    func dispatch(_ prepared: Prepared) async throws -> JSON
    func resolve(_ command: String, evidence: JSON) async -> JSON
}
