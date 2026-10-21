import Foundation

public enum CommandOrigin: Sendable, Equatable {
    case gated, interactive

    public init(_ frame: JSON) throws {
        switch frame["origin"] {
        case .null, .string(""): self = .gated
        case .string("interactive"):
            let principal = frame["principal"].string
            let parts = principal.split(separator: "/", omittingEmptySubsequences: false)
            let paired = parts.count == 2 && parts[0].hasPrefix("replica:") && parts[0].count > 8 && !principal.contains("\r") && !principal.contains("\n")
            let delegated = paired && ((parts[1].hasPrefix("device:") && parts[1].count > 7) || (parts[1].hasPrefix("agent:") && parts[1].count > 6))
            guard principal.hasPrefix("device:") || principal.hasPrefix("agent:") || delegated else { throw SourceFailure.invalid }
            self = .interactive
        default: throw SourceFailure.invalid
        }
        guard frame["action"]["origin"] == .null || frame["action"]["origin"] == frame["origin"] else { throw SourceFailure.invalid }
    }
}
