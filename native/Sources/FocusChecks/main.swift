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
        if CommandLine.arguments.last == "status" {
 guard FocusSource.status()["authorization"].string == "authorized", FocusSource.status()["focused"] != .null else { print("limited: needs Focus status grant"); return }
 print("passed"); return
 }
 if CommandLine.arguments.last == "real" {
            let before = FocusSource.status()
            guard before["authorization"].string == "authorized", before["focused"] == .bool(false) else {
                print("limited: needs Focus status grant or an initially inactive Focus")
                return
            }
            let source = FocusSource()
            let until = ISO8601DateFormatter().string(from: Date().addingTimeInterval(60))
            let on = try await source.prepare(command(on: true, until: until))
            let off = try await source.prepare(command(on: false))
            var restored = false
            do {
                let result = try await source.dispatch(on)
                guard result["readback"]["focused"] == .bool(true) else { throw SourceFailure.corrupt }
                let resultOff = try await source.restoreInactive(off)
                restored = resultOff["readback"]["focused"] == before["focused"]
                guard restored else { throw SourceFailure.corrupt }
            } catch {
                if !restored { _ = try await source.restoreInactive(off) }
                throw error
            }
            print("passed")
            return
        }
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
        fixture.authorized = false
        do { _ = try await source.prepare(command(on: true)); throw SourceFailure.corrupt } catch SourceFailure.permissionMissing {}
        guard fixture.calls == 2 else { throw SourceFailure.corrupt }
        print("passed")
    }
    static func command(on: Bool, until: String = "") -> JSON {
        .object(["request_hash": .string("synthetic"), "action": .object(["op": .string("focus.set"), "args": .object(["mode": .string("Do Not Disturb"), "on": .bool(on), "until": .string(until)])])])
    }
}
