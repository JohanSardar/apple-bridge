import Foundation
import SourceSupport
import FocusSource

@MainActor
final class Fixture {
 var focused = false
 var calls = 0
 var authorized = true
}

@main
struct Checks {
    @MainActor
    static func main() async throws {
        let fixture = Fixture()
        let source = FocusSource(status: { .object(["focused": .bool(fixture.focused), "name": .null, "authorization": .string(fixture.authorized ? "authorized" : "not_determined")]) }, runner: { args in
            fixture.calls += 1
            fixture.focused = args["on"].bool
            return .object(["exit_status": .number(0), "timed_out": .bool(false)])
        })
        let prior = fixture.focused
        let on = try await source.prepare(command(on: true, until: ISO8601DateFormatter().string(from: Date().addingTimeInterval(60))))
        let result = try await source.dispatch(on)
        guard result["observed"].bool, result["readback"]["focused"] == .bool(true), fixture.calls == 1 else { throw SourceFailure.corrupt }
        let off = try await source.prepare(command(on: prior))
        _ = try await source.restoreInactive(off)
        guard fixture.focused == prior, fixture.calls == 2 else { throw SourceFailure.corrupt }
        let unchanged = FocusSource(status: { .object(["focused": .bool(false), "authorization": .string("authorized"), "name": .null]) }, runner: { _ in .object(["exit_status": .number(0), "timed_out": .bool(false)]) })
        let unconfirmed = try await unchanged.prepare(command(on: true))
        do { _ = try await unchanged.dispatch(unconfirmed); throw SourceFailure.corrupt } catch SourceFailure.unavailable {}
        let lost = Fixture()
        let cleanup = FocusSource(status: { .object(["focused": lost.authorized ? .bool(lost.focused) : .null, "authorization": .string(lost.authorized ? "authorized" : "not_determined"), "name": .null]) }, runner: { args in
            lost.calls += 1
            lost.focused = args["on"].bool
            lost.authorized = false
            return .object(["exit_status": .number(0), "timed_out": .bool(false)])
        })
        let cleanupOff = try await cleanup.prepare(command(on: false))
        let cleanupOn = try await cleanup.prepare(command(on: true))
        do { _ = try await cleanup.dispatch(cleanupOn); throw SourceFailure.corrupt } catch SourceFailure.unavailable {}
        guard lost.focused, lost.calls == 1 else { throw SourceFailure.corrupt }
        do { _ = try await cleanup.restoreInactive(cleanupOff); throw SourceFailure.corrupt } catch SourceFailure.unavailable {}
        guard !lost.focused, lost.calls == 2 else { throw SourceFailure.corrupt }
        #if LAB
        let proofFixture = Fixture()
        let proofStatus: @MainActor () -> JSON = { .object(["focused": .bool(proofFixture.focused), "authorization": .string("authorized")]) }
        let proofSource = FocusSource(status: proofStatus, runner: { args in
            proofFixture.calls += 1
            if args["on"].bool { proofFixture.focused = true; throw SourceFailure.unavailable }
            proofFixture.focused = false
            return .object(["exit_status": .number(0), "timed_out": .bool(false)])
        })
        let failedProof = try await FocusQualification.run("FOC-SET-REAL", source: proofSource, status: proofStatus)
        guard failedProof["set_passed"] == .bool(false), failedProof["restore_passed"] == .bool(true), proofFixture.calls == 2, !proofFixture.focused else { throw SourceFailure.corrupt }
        let phoneFixture = Fixture()
        let phoneStatus: @MainActor () -> JSON = { .object(["focused": .bool(phoneFixture.focused), "authorization": .string("authorized")]) }
        let phoneSource = FocusSource(status: phoneStatus, runner: { args in
            phoneFixture.calls += 1
            phoneFixture.focused = args["on"].bool
            return .object(["exit_status": .number(0), "timed_out": .bool(false)])
        })
        var observed = false
        let phoneProof = try await FocusQualification.run("FOC-PHONE-SYNC", source: phoneSource, status: phoneStatus, observePhone: { observed = phoneFixture.focused })
        guard observed, phoneProof["set_passed"].bool, phoneProof["restore_passed"].bool, !phoneFixture.focused, phoneFixture.calls == 2 else { throw SourceFailure.corrupt }
        let interruptedPhone = try await FocusQualification.run("FOC-PHONE-SYNC", source: phoneSource, status: phoneStatus, observePhone: { throw SourceFailure.unavailable })
        guard interruptedPhone["restore_passed"].bool, !phoneFixture.focused, phoneFixture.calls == 4 else { throw SourceFailure.corrupt }
        #endif
        fixture.authorized = false
        do { _ = try await source.prepare(command(on: true)); throw SourceFailure.corrupt } catch SourceFailure.permissionMissing {}
        guard fixture.calls == 2 else { throw SourceFailure.corrupt }
        print("passed")
    }
    static func command(on: Bool, until: String = "") -> JSON {
        .object(["request_hash": .string("synthetic"), "action": .object(["op": .string("focus.set"), "args": .object(["mode": .string("Do Not Disturb"), "on": .bool(on), "until": .string(until)])])])
    }
}
