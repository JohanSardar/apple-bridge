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
    case permissionMissing, unavailable, invalid, conflict, unsupported, expired, corrupt, capabilityUnproven
}

public func timestamp(_ date: Date = Date()) -> String {
    date.ISO8601Format(.init(includingFractionalSeconds: true))
}

public func parseTimestamp(_ value: String) -> Date? {
    let format = ISO8601DateFormatter()
    format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return format.date(from: value) ?? ISO8601DateFormatter().date(from: value)
}

public protocol Source: Sendable {
    var name: String { get }
    func status() async -> JSON?
    func primitive(_ operation: String, args: JSON) async throws -> JSON
}

public extension Source {
    func primitive(_ operation: String, args: JSON) async throws -> JSON { throw SourceFailure.unsupported }
    func status() async -> JSON? { nil }
}

@globalActor
public actor UIActor {
 public static let shared = UIActor()
}

public struct SourceEnumeration: Sendable {
    public let generation: String
    public let rows: [JSON]
    public let handles: [String: String]
    public init(generation: String, rows: [JSON], handles: [String: String] = [:]) {
        self.generation = generation; self.rows = rows; self.handles = handles
    }
}

public enum RevalidationState {
    public static let present = "present"
    public static let missing = "missing"
    public static let outsideScope = "outside_scope"
}
