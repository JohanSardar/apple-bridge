import Foundation

public enum JSON: Codable, Sendable, Equatable {
    case object([String: JSON]), array([JSON]), string(String), number(Double), bool(Bool), null

    public init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let result = try? value.decode(Bool.self) { self = .bool(result) }
        else if let result = try? value.decode(String.self) { self = .string(result) }
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
        case .number(let result): try value.encode(result)
        case .bool(let result): try value.encode(result)
        case .null: try value.encodeNil()
        }
    }

    public subscript(_ name: String) -> JSON { if case .object(let value) = self { return value[name] ?? .null }; return .null }
    public var string: String { if case .string(let value) = self { return value }; return "" }
    public var object: [String: JSON] { if case .object(let value) = self { return value }; return [:] }
    public var array: [JSON] { if case .array(let value) = self { return value }; return [] }
    public var number: Double { if case .number(let value) = self { return value }; return 0 }
    public var bool: Bool { if case .bool(let value) = self { return value }; return false }
}

public enum SourceFailure: Error {
    case permissionMissing, unavailable, invalid, conflict, unsupported, expired, corrupt
}

public func timestamp(_ date: Date = Date()) -> String {
    let format = ISO8601DateFormatter()
    format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return format.string(from: date)
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
    func emit(source: String, observations: [JSON], scanStartedAt: String, scope: [String], live: Bool) async throws
}

public protocol Source: Sendable {
    var name: String { get }
    func open() async -> SourceReadiness
    func capture(_ resume: JSON, into sink: any ObservationSink) async throws
    func prepare(_ command: JSON) async throws -> Prepared
    func dispatch(_ prepared: Prepared) async throws -> JSON
    func resolve(_ command: String, evidence: JSON) async -> JSON
}
