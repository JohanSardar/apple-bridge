import Foundation
import SourceSupport
import FocusSource
import ObjectiveC

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
        let focusActions = ["detect.dictionary", "getvalueforkey", "getvalueforkey", "conditional", "dnd.set", "conditional", "dnd.set", "conditional"].map { "is.workflow.actions." + $0 }
        guard FocusPackRepair.supportedShape(focusActions),
              FocusPackRepair.supportedShape(focusActions + ["is.workflow.actions.output"]),
              !FocusPackRepair.supportedShape(focusActions + ["is.workflow.actions.runworkflow"]),
              !FocusPackRepair.supportedShape(Array(focusActions.dropLast())) else { throw SourceFailure.corrupt }
        let databaseDirectory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: databaseDirectory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: databaseDirectory) }
        _ = try FocusPackRepair.database(testURL: databaseDirectory.appendingPathComponent("focus.sqlite"))
        let attachment = JSON.object(["WFSerializationType": .string("WFTextTokenAttachment"), "Value": .object(["Type": .string("ActionOutput"), "OutputUUID": .string("synthetic-until")])])
        let repaired = try FocusPackRepair.binding(attachment, output: "synthetic-until")
        guard repaired["Type"] == .string("Variable"), repaired["Variable"] == attachment,
              try FocusPackRepair.binding(repaired, output: "synthetic-until") == repaired else { throw SourceFailure.corrupt }
        do { _ = try FocusPackRepair.binding(attachment, output: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
        let repairedDate = try FocusPackRepair.dateBinding(attachment, output: "synthetic-until")
        guard repairedDate["WFSerializationType"] == .string("WFTextTokenString"),
              repairedDate["Value"]["attachmentsByRange"]["{0, 1}"] == attachment["Value"],
              try FocusPackRepair.dateBinding(repairedDate, output: "synthetic-until") == repairedDate else { throw SourceFailure.corrupt }
        do { _ = try FocusPackRepair.dateBinding(attachment, output: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
        guard let dateClass = NSClassFromString("WFDateFieldParameterState") else { throw SourceFailure.unavailable }
        let dateSelector = NSSelectorFromString("initWithSerializedRepresentation:variableProvider:parameter:")
        guard let dateMethod = class_getInstanceMethod(dateClass, dateSelector) else { throw SourceFailure.unavailable }
        typealias ParseDate = @convention(c) (AnyObject, Selector, AnyObject, AnyObject?, AnyObject?) -> Unmanaged<AnyObject>?
        let parseDate = unsafeBitCast(method_getImplementation(dateMethod), to: ParseDate.self)
        func dateState(_ value: JSON) throws -> AnyObject? {
            let serialized = try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as AnyObject
            let allocated = (dateClass as AnyObject).perform(NSSelectorFromString("alloc"))!.takeUnretainedValue()
            return parseDate(allocated, dateSelector, serialized, nil, nil)?.takeRetainedValue()
        }
        guard try dateState(attachment) == nil, try dateState(repairedDate) != nil else { throw SourceFailure.corrupt }
        guard FocusQualification.failureCategory("An action could not be found") == "missing_action", FocusQualification.failureCategory("Cannot convert private input to a dictionary") == "dictionary_input", FocusQualification.failureCategory("opaque private contents") == "shortcut_execution" else { throw SourceFailure.corrupt }
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
