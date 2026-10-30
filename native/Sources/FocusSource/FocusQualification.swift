#if LAB
import Foundation
import SourceSupport

@MainActor
public enum FocusQualification {
    public static func run(_ scenario: String, source: FocusSource = FocusSource(), status: @MainActor () -> JSON = FocusSource.status) async throws -> JSON {
        guard ["FOC-SET-REAL", "FOC-STATUS-REAL"].contains(scenario) else { throw SourceFailure.invalid }
        let before = status()
        guard before["authorization"].string == "authorized", before["focused"] != .null else { return .object(["grant_passed": .bool(false)]) }
        if scenario == "FOC-STATUS-REAL" { return .object(["grant_passed": .bool(true), "status_passed": .bool(true)]) }
        guard before["focused"] == .bool(false) else { return .object(["grant_passed": .bool(true), "inactive_passed": .bool(false)]) }
        func command(_ on: Bool, until: String = "") -> JSON {
            .object(["request_hash": .string(UUID().uuidString), "action": .object(["op": .string("focus.set"), "args": .object(["mode": .string("Do Not Disturb"), "on": .bool(on), "until": .string(until)])])])
        }
        var checks: [String: JSON] = ["grant_passed": .bool(true), "inactive_passed": .bool(true), "set_passed": .bool(false), "restore_passed": .bool(false)]
        do {
            let off = try await source.prepare(command(false))
            do {
                let on = try await source.prepare(command(true, until: ISO8601DateFormatter().string(from: Date().addingTimeInterval(120))))
                let enabled = try await source.dispatch(on)
                checks["set_passed"] = .bool(enabled["readback"]["focused"] == .bool(true))
            } catch { checks["set_error"] = .string(String(describing: error)) }
            do {
                let disabled = try await source.restoreInactive(off)
                checks["restore_passed"] = .bool(disabled["readback"]["focused"] == before["focused"])
            } catch { checks["restore_error"] = .string(String(describing: error)) }
        } catch { checks["prepare_error"] = .string(String(describing: error)) }
        return .object(checks)
    }
}
#endif
