import Foundation
import AppKit
import ApplicationServices
import Carbon
import SourceSupport

public enum MessagesVariantProof {
    public static func run(_ kind: String) async throws -> JSON {
        guard ["audio", "formatting", "mention", "effect", "schedule"].contains(kind) else { throw SourceFailure.invalid }
        if kind == "audio" { return limited("needs native media cleanup qualification") }
        if kind == "schedule" { return limited("needs verified Send Later date controls and cancellation cleanup") }
        if kind == "mention" { return limited("needs self-chat mention suggestion identity and attribute qualification") }
        try MessagesSendVariants.invalidateProof(kind)
        guard AXIsProcessTrusted() else { return limited("needs Messages Accessibility grant") }
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        guard session?["CGSSessionScreenIsLocked"] as? Bool != true else { return limited("needs unlocked Mac") }
        await MessagesUISession.activate()
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let reader = try MessagesReader(path: path)
        guard let selected = try MessagesSelfTestIsolation.target(database: MessagesDatabase()) else { return limited("No verified self conversation is available. Check the Messages account's own addresses and self conversation.") }
        let handle = selected.handle
        let existingBaseline: MessagesSelfTestBaseline?
        do { existingBaseline = selected.chat.isEmpty ? nil : try await MessagesSelfTestBaseline.capture(selected.chat) }
        catch { return limited("The self conversation baseline could not be verified. Keep Messages available and rerun the check.") }
        var chat = ""
        let run = "variant_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let tag = "[ab-test " + run + "]"
        let baseline = try reader.maximum()
        let baselineChat = try reader.query("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM chat").first?.integer("maximum") ?? 0
        let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")
        try FileManager.default.createDirectory(at: ledger.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var registration: [String: Any] = ["run_id": run, "baseline_max_row_id": baseline, "baseline_max_chat_row_id": baselineChat, "cleanup_state": "pending_native_cleanup", "created_message_ids": [String]()]
        try PrivateReport.write(registration, to: ledger)
        let commands = MessagesCommands(path: path, labCommands: true)
        var writes = 0
        var outcome: JSON = .null
        var reason = ""
        var preparedVariant: Prepared?
        var lateEffectPossible = false
        var echoVerified = false
        let creations = URL(fileURLWithPath: ledger.path + ".creations.json")
        @Sendable func captureCreatedIDs() throws {
            let reader = try MessagesReader(path: path)
            let ids = try reader.query("SELECT guid FROM message WHERE ROWID>? AND (instr(text,?)>0 OR instr(attributedBody,CAST(? AS BLOB))>0)", [String(baseline), tag, tag]).map { $0["guid"] }
            try PrivateReport.write(["run_id": run, "ids": ids], to: creations)
        }
        let captureTask = Task.detached {
            while !Task.isCancelled {
                try? captureCreatedIDs()
                do { try await Task.sleep(for: .milliseconds(100)) } catch { break }
            }
        }
        defer { captureTask.cancel() }
        func capture() throws {
            let rows = try reader.query("SELECT guid FROM message WHERE ROWID>? AND (instr(text,?)>0 OR instr(attributedBody,CAST(? AS BLOB))>0)", [String(baseline), tag, tag])
            registration["created_message_ids"] = rows.map { $0["guid"] }
            try PrivateReport.write(registration, to: ledger)
        }
        func prepare(_ text: String, variant: JSON = .null) async throws -> Prepared {
            let action = JSON.object(["op": .string("messages.send"), "list_key": .string(chat), "messages": .object(["text": .string(text), "variant": variant])])
            return try await commands.prepare(.object(["command_id": .string(UUID().uuidString), "principal": .string("selftest"), "request_hash": .string(UUID().uuidString), "expires_at": .string(timestamp(Date().addingTimeInterval(30))), "action": action]))
        }
        do {
            writes += 1
            lateEffectPossible = true
            let initial = try await MessagesParityFixture.send(text: tag + " fixture", handle: handle, existingChat: selected.chat)
            lateEffectPossible = initial["disposition"].string == "indeterminate"
            try capture()
            guard initial["observed"].bool else { throw SourceFailure.capabilityUnproven }
            chat = initial["chat_key"].string
            guard let created = try reader.query("SELECT ROWID AS position FROM chat WHERE guid=?", [chat]).first,
                  (selected.chat.isEmpty ? created.integer("position") > baselineChat : chat == selected.chat) else { throw SourceFailure.conflict }
            registration["created_chat_key"] = chat
            registration["created_by_run"] = selected.chat.isEmpty
            registration["existing_self_chat"] = !selected.chat.isEmpty
            try capture()
            let text = tag + " variant"
            let variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([.object(["utf16_location": .number(Double(tag.utf16.count + 1)), "utf16_length": .number(7), "styles": .array([.string("bold")])])])])
            let prepared = try await prepare(text, variant: variant)
            preparedVariant = prepared
            try await commands.validate(prepared)
            writes += 1
            outcome = try await commands.dispatch(prepared)
            lateEffectPossible = lateEffectPossible || outcome["disposition"].string == "indeterminate"
            reason = outcome["reason"].string
            if outcome["observed"].bool, outcome["disposition"].string == "locally_observed" {
                let deadline = ContinuousClock.now.advanced(by: .seconds(10))
                repeat {
                    let echoes = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(baseline), chat, text, text])
                    if echoes.count == 1, let row = try reader.message(echoes[0]["guid"]), let captured = try reader.messageObservations(row).first {
                        let data = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: captured.2))
                        echoVerified = MessagesSendVariants.matchesEcho(prepared.action["messages"], data: data)
                        if echoVerified { break }
                    }
                    try await Task.sleep(for: .milliseconds(100))
                } while ContinuousClock.now < deadline
                if !echoVerified { reason = "self_echo_variant_attributes_unverified" }
            }
        } catch { reason = (error as? SendVariantFailure)?.rawValue ?? (error as? MessagesUIFailure)?.rawValue ?? (error is SourceFailure ? String(describing: error) : "native_variant_probe_failed") }
        try await Task.sleep(for: .seconds(1))
        var ownershipCaptured = false
        do { try capture(); ownershipCaptured = true } catch { reason = "created_ids_capture_failed" }
        var cleanup: JSON = .null
        for _ in 0..<3 {
            try capture()
            try captureCreatedIDs()
            let cleaned = try await MessagesCleanup.run(["--tag", tag, "--owned-only"])
            cleanup = try JSONDecoder().decode(JSON.self, from: cleaned)
            try await Task.sleep(for: .milliseconds(500))
        }
        let remaining = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.ROWID>? AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0) AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=j.chat_id)", [String(baseline), tag, tag])
        let baselineRestored = (try? await existingBaseline?.restoreAndVerify()) ?? (existingBaseline == nil)
        let cleanupPassed = baselineRestored && ownershipCaptured && cleanup["passed"].bool && remaining.isEmpty
        let passed = cleanupPassed && !lateEffectPossible && echoVerified && outcome["observed"].bool && outcome["disposition"].string == "locally_observed"
        registration["late_effect_possible"] = lateEffectPossible
        registration["cleanup_state"] = cleanupPassed ? (lateEffectPossible ? "pending_reconciliation" : "verified_clean") : "pending_native_cleanup"
        try PrivateReport.write(registration, to: ledger)
        if passed, let prepared = preparedVariant {
            try MessagesSendVariants.recordProof(prepared)
        }
        return .object(["passed": .bool(passed), "limited": .bool(!passed && cleanupPassed), "reason": .string(reason.isEmpty ? "needs native sent-attribute qualification" : reason), "cleanup_passed": .bool(cleanupPassed), "writes_submitted": .number(Double(writes))])
    }
    static func limited(_ reason: String) -> JSON {
        .object(["passed": .bool(false), "limited": .bool(true), "reason": .string(reason), "writes_submitted": .number(0)])
    }
}
