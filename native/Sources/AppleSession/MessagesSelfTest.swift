import Foundation
import Darwin
import CoreGraphics
import MessagesSource
import Decoders
import SourceSupport

actor MessagesSelfTest {
    static let shared = MessagesSelfTest()
    private var active = false
    private let build = ProcessInfo.processInfo.operatingSystemVersionString
    private var reportURL: URL { PrivateReport.directory.appendingPathComponent("messages-selftest.json") }

    private func failureReason(_ error: Error) -> String {
        if let error = error as? SourceFailure { return String(describing: error) }
        if let error = error as? MessagesUIFailure { return error.rawValue }
        if let error = error as? ProbeFailure { return String(describing: error) }
        let native = error as NSError
        return native.domain + ":" + String(native.code)
    }

    func run(force: Bool = false, includeUI: Bool = true) async throws -> JSON {
        guard !active else { throw SourceFailure.conflict }
        active = true
        defer { active = false }
        let home = PrivateReport.directory.appendingPathComponent("messages-selftest", isDirectory: true)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let descriptor = Darwin.open(home.appendingPathComponent("selftest.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        defer { Darwin.close(descriptor) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600 else { throw SourceFailure.invalid }
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw SourceFailure.conflict }
        if !force, let data = try? Data(contentsOf: reportURL), let old = try? JSONDecoder().decode(JSON.self, from: data), old["os_build"].string == build { return old }
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        let uiAllowed = includeUI && session?["CGSSessionScreenIsLocked"] as? Bool != true
        let run = "selftest_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let tag = "[ab-test " + run + "]"
        let guardLog = try DispatchGuard(home: home)
        let commands = MessagesCommands(path: nil, labCommands: true)
        let handles = try MessagesProbes.discoverSelfHandles().handles
        let database = try MessagesDatabase()
        let chats = try database.rows("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC")
        guard let handle = chats.compactMap({ $0["id"] }).first(where: { handles.contains(MessagesProbes.normalize($0)) }) ?? handles.first else { throw SourceFailure.invalid }
        let baseline = try database.rows("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM message").first?["maximum"] ?? "0"
        let cleanup = PrivateReport.directory.appendingPathComponent("messages-command-cleanup", isDirectory: true)
        try FileManager.default.createDirectory(at: cleanup, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let filename = "ab-" + run + ".txt"
        let ledger = cleanup.appendingPathComponent(run + ".json")
        var registration: [String: Any] = ["scenario": "MSG-SELFTEST", "run_id": run, "baseline_max_row_id": Int64(baseline) ?? 0, "media_attachment_names": [filename], "cleanup_state": "pending_native_cleanup"]
        try PrivateReport.write(registration, to: ledger)
        var steps: [JSON] = []
        var lateEffectPossible = false
        func save(_ state: String) throws -> JSON {
            let result = JSON.object(["os_build": .string(build), "run_id": .string(run), "state": .string(state), "steps": .array(steps)])
            try JSONEncoder().encode(result).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return result
        }
        _ = try save("running")
        func execute(_ name: String, _ operation: String, args: [String: JSON], native: String = "") async -> JSON {
            let id = UUID().uuidString
            let started = ContinuousClock.now
            let expiry = Date().addingTimeInterval(30)
            let deadline = ContinuousClock.now.advanced(by: .seconds(30))
            let action = JSON.object(["op": .string("messages." + operation), "messages": .object(args), "native_key": .string(native)])
            let frame = JSON.object(["command_id": .string(id), "principal": .string("selftest"), "request_hash": .string(UUID().uuidString), "action": action, "expires_at": .string(timestamp(expiry))])
            var result: JSON = .null
            var reason = ""
            var dispatched = false
            do {
                guard !(await guardLog.contains(id)) else { throw SourceFailure.conflict }
                let prepared = try await commands.prepare(frame)
                try await commands.validate(prepared)
                guard expiry > Date() else { throw SourceFailure.expired }
                try await guardLog.append(.object(["t": .string("Started"), "command_id": .string(id), "prepared": .object(["action": prepared.action, "native_state": prepared.nativeState])]))
                guard expiry > Date(), ContinuousClock.now < deadline else { throw SourceFailure.expired }
                dispatched = true
                result = try await commands.dispatch(prepared)
                lateEffectPossible = lateEffectPossible || result["disposition"].string == "indeterminate"
                try await guardLog.append(.object(["t": .string("Outcome"), "command_id": .string(id), "disposition": result["disposition"], "evidence": result]))
                if !result["observed"].bool || !["locally_observed", "delivered"].contains(result["disposition"].string) {
                    reason = result["disposition"].string
                    var failed = result.object
                    failed["observed"] = .bool(false)
                    result = .object(failed)
                }
            } catch {
                reason = failureReason(error)
                lateEffectPossible = lateEffectPossible || dispatched
                var failed = result.object
                failed["observed"] = .bool(false)
                failed["disposition"] = .string(dispatched ? "indeterminate" : "rejected_before_dispatch")
                result = .object(failed)
            }
            let duration = started.duration(to: ContinuousClock.now)
            let milliseconds = Double(duration.components.seconds) * 1000 + Double(duration.components.attoseconds) / 1e15
            steps.append(.object(["name": .string(name), "operation": .string("messages." + operation), "proven": .bool(result["observed"].bool), "reason": .string(reason), "latency_ms": .number(milliseconds)]))
            _ = try? save("running")
            return result
        }
        let send: [String: JSON] = ["handles": .array([.string(handle)]), "service": .string("iMessage"), "text": .string(tag + " self-test")]
        let initial = await execute("send_text", "send", args: send)
        let target = initial["native_key"].string
        if uiAllowed, initial["observed"].bool {
            if let chat = try? database.rows("SELECT c.guid FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", bindings: [target]).first?["guid"] {
                _ = await execute("mark_unread", "mark_unread", args: [:], native: chat)
                _ = await execute("mark_read", "mark_read", args: [:], native: chat)
            }
        }
        do {
            let uploads = home.appendingPathComponent("message-uploads", isDirectory: true)
            try FileManager.default.createDirectory(at: uploads, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let upload = uploads.appendingPathComponent(filename)
            let bytes = Data((tag + " synthetic file").utf8)
            try bytes.write(to: upload, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: upload.path)
            var fileArgs = send
            fileArgs["text"] = .string("")
            fileArgs["files"] = .array([.object(["name": .string(filename), "path": .string(upload.path), "digest": .string(BodyDigest.hash(bytes))])])
            _ = await execute("send_file", "send", args: fileArgs)
            try? FileManager.default.removeItem(at: upload)
        } catch {
            steps.append(.object(["name": .string("send_file"), "operation": .string("messages.send"), "proven": .bool(false), "reason": .string(failureReason(error))]))
        }
        if uiAllowed, initial["observed"].bool, !target.isEmpty {
            _ = await execute("reply", "reply", args: ["text": .string(tag + " reply")], native: target)
            for (kind, emoji) in [("love", ""), ("emoji", "😂")] {
                let added = await execute(kind + "_add", "react", args: ["reaction": .string(kind), "emoji": .string(emoji), "remove": .bool(false)], native: target)
                if added["observed"].bool {
                    _ = await execute(kind + "_remove", "react", args: ["reaction": .string(kind), "emoji": .string(emoji), "remove": .bool(true)], native: target)
                } else {
                    steps.append(.object(["name": .string(kind + "_remove"), "operation": .string("messages.react"), "proven": .bool(false), "reason": .string("reaction_add_unproven")]))
                }
            }
            _ = await execute("edit", "edit", args: ["text": .string(tag + " edited")], native: target)
            _ = await execute("delete_message", "delete_message", args: [:], native: target)
        }
        if uiAllowed, initial["observed"].bool {
            var unsend = send
            unsend["text"] = .string(tag + " unsend")
            let retract = await execute("unsend_fixture", "send", args: unsend)
            if retract["observed"].bool, !retract["native_key"].string.isEmpty { _ = await execute("unsend", "unsend", args: [:], native: retract["native_key"].string) }
        }
        for (name, operation) in [("reply", "reply"), ("love_add", "react"), ("love_remove", "react"), ("emoji_add", "react"), ("emoji_remove", "react"), ("edit", "edit"), ("unsend", "unsend"), ("mark_read", "mark_read"), ("mark_unread", "mark_unread"), ("delete_message", "delete_message")] where !steps.contains(where: { $0["name"].string == name }) {
            steps.append(.object(["name": .string(name), "operation": .string("messages." + operation), "proven": .bool(false), "reason": .string(uiAllowed ? "prerequisite_send_unproven" : "ui_deferred_until_unlock")]))
        }
        let failedKinds = Set(steps.filter { !$0["proven"].bool }.map { step in
            step["operation"].string == "messages.send" ? (step["name"].string == "send_file" ? "attachment" : "text") : step["operation"].string
        })
        for kind in failedKinds where !kind.isEmpty {
            do { try await commands.invalidateProof(kind) }
            catch { steps.append(.object(["name": .string("capability_evidence"), "proven": .bool(false), "reason": .string(failureReason(error))])) }
        }
        do {
            guard uiAllowed else { throw SourceFailure.unavailable }
            let cleanup = try await MessagesCleanup.run(["--tag", tag])
            let result = try JSONDecoder().decode(JSON.self, from: cleanup)
            steps.append(.object(["name": .string("cleanup"), "proven": result["passed"], "reason": result["failure"]]))
            registration["cleanup_state"] = result["passed"].bool ? (lateEffectPossible ? "pending_reconciliation" : "verified_clean") : "pending_native_cleanup"
        } catch {
            steps.append(.object(["name": .string("cleanup"), "proven": .bool(false), "reason": .string(uiAllowed ? failureReason(error) : "ui_deferred_until_unlock")]))
        }
        registration["late_effect_possible"] = lateEffectPossible
        try PrivateReport.write(registration, to: ledger)
        return try save(steps.allSatisfy { $0["proven"].bool } ? "proven" : "limited")
    }
}
