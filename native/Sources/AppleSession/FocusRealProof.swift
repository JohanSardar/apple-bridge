#if LAB
import Foundation
import FocusSource
import SourceSupport

@MainActor
enum FocusRealProof {
    static func run(_ scenario: String) async throws -> JSON {
        guard ["FOC-SET-REAL", "FOC-STATUS-REAL"].contains(scenario) else { throw SourceFailure.invalid }
        let before = FocusSource.status()
        guard before["authorization"].string == "authorized", before["focused"] != .null else { return .object(["grant_passed": .bool(false)]) }
        if scenario == "FOC-STATUS-REAL" { return .object(["grant_passed": .bool(true), "status_passed": .bool(true)]) }
        guard before["focused"] == .bool(false) else { return .object(["grant_passed": .bool(true), "inactive_passed": .bool(false)]) }
        let source = FocusSource()
        func command(_ on: Bool, until: String = "") -> JSON {
            .object(["request_hash": .string(UUID().uuidString), "action": .object(["op": .string("focus.set"), "args": .object(["mode": .string("Do Not Disturb"), "on": .bool(on), "until": .string(until)])])])
        }
        let on = try await source.prepare(command(true, until: ISO8601DateFormatter().string(from: Date().addingTimeInterval(60))))
        let off = try await source.prepare(command(false))
        var restored = false
        do {
            let enabled = try await source.dispatch(on)
            guard enabled["readback"]["focused"] == .bool(true) else { throw SourceFailure.corrupt }
            let disabled = try await source.restoreInactive(off)
            restored = disabled["readback"]["focused"] == before["focused"]
            guard restored else { throw SourceFailure.corrupt }
        } catch {
            if !restored { _ = try await source.restoreInactive(off) }
            throw error
        }
        return .object(["grant_passed": .bool(true), "inactive_passed": .bool(true), "set_passed": .bool(true), "restore_passed": .bool(true)])
    }
}
#endif
