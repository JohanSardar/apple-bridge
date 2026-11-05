import Foundation
import Intents
import SourceSupport

@MainActor
public final class FocusSource: Source {
    public nonisolated let name = "focus"
    public init() {}
    public static func frameworkStatus() -> JSON {
        let center = INFocusStatusCenter.default
        let authorization: String
        switch center.authorizationStatus {
        case .authorized: authorization = "authorized"
        case .denied: authorization = "denied"
        case .restricted: authorization = "restricted"
        default: authorization = "not_determined"
        }
        return .object(["authorization": .string(authorization), "center_focused": center.authorizationStatus == .authorized ? center.focusStatus.isFocused.map { .bool($0) } ?? .null : .null])
    }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard operation == "focus.framework_status" else { throw SourceFailure.unsupported }
        return Self.frameworkStatus()
    }
}
