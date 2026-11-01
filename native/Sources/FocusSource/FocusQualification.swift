#if LAB
import Foundation
import SourceSupport
import ShortcutsSource

@MainActor
public enum FocusQualification {
    public static func failureCategory(_ text: String) -> String {
        let text = text.lowercased()
        if text.isEmpty { return "empty_error" }
        for (label, words) in [("missing_action", ["action", "not be found"]), ("missing_action", ["action", "not found"]), ("unsupported_action", ["action", "not available"]), ("permission", ["permission"]), ("untrusted", ["untrusted"]), ("dictionary_input", ["dictionary"]), ("input_conversion", ["convert"]), ("missing_parameter", ["parameter"]), ("focus_action", ["focus"]), ("file_access", ["file"]), ("shortcut_not_found", ["shortcut", "not found"])] {
            if words.allSatisfy({ text.contains($0) }) { return label }
        }
        return "shortcut_execution"
    }
    public static func run(_ scenario: String, source: FocusSource? = nil, status: @escaping @MainActor () -> JSON = FocusSource.status, observePhone: @MainActor () async throws -> Void = { try await Task.sleep(for: .seconds(15)) }) async throws -> JSON {
        guard ["FOC-SET-REAL", "FOC-STATUS-REAL", "FOC-PHONE-SYNC"].contains(scenario) else { throw SourceFailure.invalid }
        let before = status()
        guard before["authorization"].string == "authorized", before["focused"] != .null else { return .object(["grant_passed": .bool(false), "grant_authorized": .bool(before["authorization"].string == "authorized"), "grant_determined": .bool(before["authorization"].string != "not_determined"), "status_known": .bool(before["focused"] != .null)]) }
        if scenario == "FOC-STATUS-REAL" { return .object(["grant_passed": .bool(true), "status_passed": .bool(true)]) }
        guard before["focused"] == .bool(false) else { return .object(["grant_passed": .bool(true), "inactive_passed": .bool(false)]) }
        func command(_ on: Bool, until: String = "") -> JSON {
            .object(["request_hash": .string(UUID().uuidString), "action": .object(["op": .string("focus.set"), "args": .object(["mode": .string("Do Not Disturb"), "on": .bool(on), "until": .string(until)])])])
        }
        var checks: [String: JSON] = ["grant_passed": .bool(true), "inactive_passed": .bool(true), "set_passed": .bool(false), "restore_passed": .bool(false)]
        var runnerChecks: [String: JSON] = [:]
        if source == nil {
            let repair = FocusPackRepair.run()
            checks["pack_binding_readback"] = repair["passed"]
            if !repair["passed"].bool {
                checks["pack_repair_error"] = repair["phase"]
                return .object(checks)
            }
        }
        let source = source ?? FocusSource(runner: { args in
            let prefix = args["on"].bool ? "set_" : "restore_"
            runnerChecks[prefix + "shortcut_completed"] = .bool(false)
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
            defer { try? FileManager.default.removeItem(at: directory) }
            let wrapper = directory.appendingPathComponent("run")
            try Data("#!/bin/sh\nexec /usr/bin/shortcuts \"$@\" 2>\"${0%/*}/stderr\"\n".utf8).write(to: wrapper)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: wrapper.path)
            let result = try await FocusSource.runShortcut(args, source: ShortcutsSource(executable: wrapper))
            runnerChecks[prefix + "shortcut_completed"] = .bool(result["exit_status"] == .number(0))
            runnerChecks[prefix + "shortcut_not_timed_out"] = .bool(!result["timed_out"].bool)
            var observed = status()
            for _ in 0..<50 {
                if observed["authorization"].string == "authorized", observed["focused"] == args["on"] { break }
                try await Task.sleep(for: .milliseconds(200))
                observed = status()
            }
            runnerChecks[prefix + "status_after_shortcut"] = .bool(observed["authorization"].string == "authorized" && observed["focused"] == args["on"])
            if result["exit_status"] != .number(0) {
                let data = try Data(contentsOf: directory.appendingPathComponent("stderr"))
                runnerChecks[prefix + "cli_error_category"] = .string(failureCategory(String(decoding: data, as: UTF8.self)))
                runnerChecks[prefix + "cli_error_bytes"] = .number(Double(data.count))
            }
            return result
        })
        do {
            let off = try await source.prepare(command(false))
            do {
                let on = try await source.prepare(command(true, until: ISO8601DateFormatter().string(from: Date().addingTimeInterval(120))))
                let enabled = try await source.dispatch(on)
                checks["set_passed"] = .bool(enabled["readback"]["focused"] == .bool(true))
                if scenario == "FOC-PHONE-SYNC" { try await observePhone() }
            } catch { checks["set_error"] = .string(String(describing: error)) }
            do {
                let disabled = try await source.restoreInactive(off)
                checks["restore_passed"] = .bool(disabled["readback"]["focused"] == before["focused"])
            } catch { checks["restore_error"] = .string(String(describing: error)) }
        } catch { checks["prepare_error"] = .string(String(describing: error)) }
        return .object(checks.merging(runnerChecks) { _, value in value })
    }
}
#endif
