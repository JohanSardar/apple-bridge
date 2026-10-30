import Foundation
import AppKit
import ApplicationServices
import Carbon
import AVFoundation
import SourceSupport

public enum MessagesVariantProof {
    public static func run(_ kind: String) async throws -> JSON {
        guard ["audio", "formatting", "mention", "effect", "schedule"].contains(kind) else { throw SourceFailure.invalid }
        try MessagesSendVariants.invalidateProof(kind)
        guard AXIsProcessTrusted() else { return limited("needs Messages Accessibility grant") }
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        guard session?["CGSSessionScreenIsLocked"] as? Bool != true else { return limited("needs unlocked Mac") }
        for bundle in ["com.apple.MobileSMS"] {
            let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { return limited("needs Messages Automation grant") }
        }
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let reader = try MessagesReader(path: path)
        let own = Set(try MessagesProbes.discoverSelfHandles().handles.map(MessagesProbes.normalize))
        let chats = try reader.query("SELECT c.guid,c.ROWID AS chat_row_id,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC")
        guard let selectedChat = chats.first(where: { own.contains(MessagesProbes.normalize($0["id"])) }) else { return limited("needs existing self-chat") }
        let chat = selectedChat["guid"]
        let run = "variant_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let tag = "[ab-test " + run + "]"
        let baseline = try reader.maximum()
        let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")
        try FileManager.default.createDirectory(at: ledger.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var registration: [String: Any] = ["scenario": kind == "audio" ? "MSG-AUDIO-MESSAGE" : "MSG-SEND-VARIANTS", "run_id": run, "baseline_max_row_id": baseline, "cleanup_state": "pending_native_cleanup", "created_message_ids": [String]()]
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
            let seed = try await prepare(tag + " fixture")
            try await commands.validate(seed)
            writes += 1
            let initial = try await commands.dispatch(seed)
            lateEffectPossible = initial["disposition"].string == "indeterminate"
            try capture()
            guard initial["observed"].bool else { throw SourceFailure.capabilityUnproven }
            let text = kind == "audio" ? "" : kind == "mention" ? tag + " @" + selectedChat["id"] : tag + " variant"
            var variant: JSON = kind == "effect" ? .object(["kind": .string(kind), "effect": .string("loud")]) : .object(["kind": .string(kind), "formatting": .array([.object(["utf16_location": .number(Double(tag.utf16.count + 1)), "utf16_length": .number(7), "styles": .array([.string("bold")])])])])
            if kind == "audio" { variant = .object(["kind": .string(kind)]) }
            else if kind == "schedule" {
                let minute = Date(timeIntervalSince1970: floor(Date().addingTimeInterval(3600).timeIntervalSince1970 / 60) * 60)
                variant = .object(["kind": .string(kind), "scheduled_for": .string(timestamp(minute))])
            } else if kind == "mention" {
                variant = .object(["kind": .string(kind), "handle_id": .string(selectedChat["id"]), "utf16_location": .number(Double(tag.utf16.count + 1)), "utf16_length": .number(Double(selectedChat["id"].utf16.count + 1))])
            }
            let prepared = try await prepare(text, variant: variant)
            preparedVariant = prepared
            try await commands.validate(prepared)
            writes += 1
            outcome = try await commands.dispatch(prepared)
            lateEffectPossible = lateEffectPossible || outcome["disposition"].string == "indeterminate"
            reason = outcome["reason"].string
        } catch { reason = (error as? SendVariantFailure)?.rawValue ?? (error as? MessagesUIFailure)?.rawValue ?? (error is SourceFailure ? String(describing: error) : "native_variant_probe_failed") }
        try await Task.sleep(for: .seconds(1))
        var ownershipCaptured = false
        do { try capture(); ownershipCaptured = true } catch { reason = "created_ids_capture_failed" }
        let cleaned = try await MessagesCleanup.run(["--tag", tag, "--owned-only", "--only-chat-id", selectedChat["chat_row_id"]])
        let cleanup = try JSONDecoder().decode(JSON.self, from: cleaned)
        let remaining = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.ROWID>? AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0) AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=j.chat_id)", [String(baseline), tag, tag])
        let cleanupPassed = ownershipCaptured && cleanup["passed"].bool && remaining.isEmpty
        let passed = cleanupPassed && outcome["observed"].bool && outcome["disposition"].string == "locally_observed"
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
