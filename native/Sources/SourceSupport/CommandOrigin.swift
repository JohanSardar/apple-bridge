import Foundation

public enum CommandOrigin: Sendable, Equatable {
    case gated, interactive

    public init(_ frame: JSON) throws {
        switch frame["origin"] {
        case .null, .string(""): self = .gated
        case .string("interactive"):
            let principal = frame["principal"].string
            guard principal.hasPrefix("device:") || principal.hasPrefix("agent:") || principal.contains("/device:") else { throw SourceFailure.invalid }
            self = .interactive
        default: throw SourceFailure.invalid
        }
        guard frame["action"]["origin"] == .null || frame["action"]["origin"] == frame["origin"] else { throw SourceFailure.invalid }
    }
}
