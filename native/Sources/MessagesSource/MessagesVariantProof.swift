import Foundation
import AppKit
import ApplicationServices
import Carbon
import AVFoundation
import SourceSupport

public enum MessagesVariantProof {
    public static func requiresSelfEcho(_ kind: String) -> Bool { kind != "schedule" }
    public static func run(_ kind: String) async throws -> JSON {
        guard ["audio", "formatting", "mention", "effect", "schedule"].contains(kind) else { throw SourceFailure.invalid }
        try MessagesSendVariants.invalidateProof(kind)
        guard AXIsProcessTrusted() else { return limited("needs Messages Accessibility grant") }
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        guard session?["CGSSessionScreenIsLocked"] as? Bool != true else { return limited("needs unlocked Mac") }
        await MessagesUISession.activate()
        if kind == "audio" {
            let bundle = "com.apple.MobileSMS"
            let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { return limited("needs Messages Automation grant") }
        }
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let reader = try MessagesReader(path: path)
        guard let selected = try MessagesSelfTestIsolation.target(database: MessagesDatabase()) else { return limited("No verified self conversation is available. Check the Messages account's own addresses and self conversation.") }
        let handle = selected.handle
        let existingBaseline: MessagesSelfTestBaseline?
        do { existingBaseline = selected.chat.isEmpty ? nil : try await MessagesSelfTestBaseline.capture(selected.chat) }
        catch { return limited((error as? MessagesSelfTestBaselineFailure)?.description ?? "The self conversation baseline could not be verified. Keep Messages available and rerun the check.") }
        var chat = ""
        let run = "variant_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let tag = "[ab-test " + run + "]"
        let baseline = try reader.maximum()
        let baselineChat = try reader.query("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM chat").first?.integer("maximum") ?? 0
        let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")
        try FileManager.default.createDirectory(at: ledger.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var registration: [String: Any] = ["scenario": kind == "audio" ? "MSG-AUDIO-MESSAGE" : "MSG-SEND-VARIANTS", "run_id": run, "baseline_max_row_id": baseline, "baseline_max_chat_row_id": baselineChat, "cleanup_state": "pending_native_cleanup", "created_message_ids": [String]()]
        try PrivateReport.write(registration, to: ledger)
        let commands = MessagesCommands(path: path, labCommands: true)
        let uploadDirectory = PrivateReport.directory.appendingPathComponent("variant-" + run + "/message-uploads")
        defer { try? FileManager.default.removeItem(at: uploadDirectory.deletingLastPathComponent()) }
        var audioFiles: [JSON] = []
        if kind == "audio" {
            try FileManager.default.createDirectory(at: uploadDirectory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let file = uploadDirectory.appendingPathComponent("ab-" + run + ".caf")
            guard let format = AVAudioFormat(standardFormatWithSampleRate: 16000, channels: 1), let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16000) else { throw SourceFailure.unavailable }
            buffer.frameLength = 16000
            guard let samples = buffer.floatChannelData else { throw SourceFailure.unavailable }
            for index in 0..<16000 { samples[0][index] = Float(sin(Double(index) * 2 * .pi * 440 / 16000) * 0.01) }
            do {
                let output = try AVAudioFile(forWriting: file, settings: format.settings)
                try output.write(from: buffer)
            }
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
            audioFiles = [.object(["name": .string(file.lastPathComponent), "path": .string(file.path), "digest": .string(try BlobDigest.file(file))])]
            registration["media_attachment_names"] = [file.lastPathComponent]
            try PrivateReport.write(registration, to: ledger)
        }
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
            var ids = Set(rows.map { $0["guid"] })
            if kind == "audio" {
                let files = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id WHERE m.ROWID>? AND (instr(a.filename,?)>0 OR instr(a.transfer_name,?)>0)", [String(baseline), "ab-" + run + ".caf", "ab-" + run + ".caf"])
                ids.formUnion(files.map { $0["guid"] })
            }
            registration["created_message_ids"] = ids.sorted()
            try PrivateReport.write(registration, to: ledger)
        }
        func prepare(_ text: String, variant: JSON = .null) async throws -> Prepared {
            var arguments: [String: JSON] = ["text": .string(text), "variant": variant]
            if variant["kind"].string == "audio" { arguments["files"] = .array(audioFiles) }
            let action = JSON.object(["op": .string("messages.send"), "list_key": .string(chat), "messages": .object(arguments)])
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
            let text = kind == "audio" ? "" : kind == "mention" ? tag + " @" + handle : tag + " variant"
            var variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([.object(["utf16_location": .number(Double(tag.utf16.count + 1)), "utf16_length": .number(7), "styles": .array([.string("bold")])])])])
            if kind == "audio" { variant = .object(["kind": .string(kind)]) }
            else if kind == "schedule" {
                let minute = Date(timeIntervalSince1970: floor(Date().addingTimeInterval(3600).timeIntervalSince1970 / 60) * 60)
                variant = .object(["kind": .string(kind), "scheduled_for": .string(timestamp(minute))])
            } else if kind == "mention" {
                variant = .object(["kind": .string(kind), "handle_id": .string(handle), "utf16_location": .number(Double(tag.utf16.count + 1)), "utf16_length": .number(Double(handle.utf16.count + 1))])
            }
            let prepared = try await prepare(text, variant: variant)
            preparedVariant = prepared
            try await commands.validate(prepared)
            let variantBaseline = try reader.maximum()
            writes += 1
            outcome = try await commands.dispatch(prepared)
            lateEffectPossible = lateEffectPossible || outcome["disposition"].string == "indeterminate"
            reason = outcome["reason"].string
            if outcome["observed"].bool, outcome["disposition"].string == "locally_observed" {
                echoVerified = !requiresSelfEcho(kind)
                let deadline = ContinuousClock.now.advanced(by: .seconds(10))
                while requiresSelfEcho(kind), ContinuousClock.now < deadline {
                    let echoText = kind == "mention" ? tag : text
                    let echoes: [SQLiteRow]
                    if kind == "audio" {
                        echoes = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id JOIN message_attachment_join a ON a.message_id=m.ROWID JOIN attachment f ON f.ROWID=a.attachment_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (instr(f.filename,?)>0 OR instr(f.transfer_name,?)>0)", [String(variantBaseline), chat, "ab-" + run + ".caf", "ab-" + run + ".caf"])
                    } else {
                        echoes = try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(variantBaseline), chat, echoText, echoText])
                    }
                    var matches = 0
                    for echo in echoes {
                        guard let row = try reader.message(echo["guid"]), let captured = try reader.messageObservations(row).first else { continue }
                        let data = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: captured.2))
                        if MessagesSendVariants.matchesEcho(prepared.action["messages"], data: data) { matches += 1 }
                    }
                    echoVerified = matches == 1
                    if echoVerified { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
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
