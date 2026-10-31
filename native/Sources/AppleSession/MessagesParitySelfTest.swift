import Foundation
import Darwin
import CoreGraphics
import ApplicationServices
import AppKit
import Carbon
import MessagesSource
import SourceSupport

actor MessagesParitySelfTest {
    static let shared = MessagesParitySelfTest()
    private var active = false
    func run(force: Bool = false, includeUI: Bool = true) async throws -> JSON {
        guard !active else { throw SourceFailure.conflict }
        active = true
        defer { active = false }
        let home = PrivateReport.directory.appendingPathComponent("messages-parity-selftest", isDirectory: true)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let sharedHome = PrivateReport.directory.appendingPathComponent("messages-selftest", isDirectory: true)
        try FileManager.default.createDirectory(at: sharedHome, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let descriptor = Darwin.open(sharedHome.appendingPathComponent("selftest.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        defer { Darwin.close(descriptor) }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600, flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw SourceFailure.conflict }
        let run = "parity_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let tag = "[ab-test " + run + "]"
        var steps: [JSON] = []
        var cleanup = "not_needed"
        var lateEffectPossible = false
        var prerequisiteReason: String?
        var beforeIDs: [String] = []
        var afterIDs: [String] = []
        var writesSubmitted = 0
        var baselineCaptured = false
        var beforePins: [String]?
        func activeIDs(_ database: MessagesDatabase) throws -> [String] {
            try database.rows("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID) ORDER BY m.guid").compactMap { $0["guid"] }
        }
        func blocked(_ reason: String) {
            steps = MessagesParity.operations.map { .object(["operation": .string($0), "proven": .bool(false), "reason": .string(reason)]) }
        }
        func save() throws -> JSON {
            let proven = baselineCaptured && cleanup == "verified_clean" && beforeIDs == afterIDs && steps.count == MessagesParity.operations.count && steps.allSatisfy { $0["proven"].bool }
            let result = JSON.object(["os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString), "helper_digest": .string(MessagesParity.buildDigest), "at": .string(timestamp()), "run_id": .string(run), "state": .string(proven ? "proven" : "limited"), "cleanup_state": .string(cleanup), "writes_submitted": .number(Double(writesSubmitted)), "before_count": .number(Double(beforeIDs.count)), "after_count": .number(Double(afterIDs.count)), "baseline_captured": .bool(baselineCaptured), "baseline_preserved": .bool(beforeIDs == afterIDs), "steps": .array(steps)])
            try JSONEncoder().encode(result).write(to: MessagesParity.reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: MessagesParity.reportURL.path)
            return result
        }
        let cached = MessagesParity.report()
        blocked("selftest_in_progress")
        _ = try save()
        steps = []
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        guard includeUI else { blocked("ui_disabled_by_caller"); return try save() }
        guard session?["CGSSessionScreenIsLocked"] as? Bool != true else { blocked("screen_locked"); return try save() }
        guard AXIsProcessTrusted() else { blocked("accessibility_permission_missing"); return try save() }
        await MessagesUISession.activate()
        if !force, cached["cleanup_state"].string == "verified_clean", cached["baseline_captured"].bool, cached["baseline_preserved"].bool,
           let at = parseTimestamp(cached["at"].string), Date().timeIntervalSince(at) >= 0, Date().timeIntervalSince(at) < 300 {
            try JSONEncoder().encode(cached).write(to: MessagesParity.reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: MessagesParity.reportURL.path)
            return cached
        }
        let database: MessagesDatabase
        let selected: (handle: String, chat: String)?
        do { database = try MessagesDatabase(); selected = try MessagesSelfTestIsolation.target(database: database) }
        catch { blocked("messages_database_or_self_handle_unavailable"); return try save() }
        guard let selected else { blocked("verified_self_chat_unavailable"); return try save() }
        let handle = selected.handle
        let existingBaseline: MessagesSelfTestBaseline?
        do { existingBaseline = selected.chat.isEmpty ? nil : try await MessagesSelfTestBaseline.capture(selected.chat) }
        catch { blocked("self_chat_baseline_" + MessagesParity.failureReason(error)); return try save() }
        beforeIDs = try activeIDs(database)
        beforePins = try? MessagesPins.read()
        baselineCaptured = true
        let baseline = Int64(try database.rows("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM chat").first?["maximum"] ?? "0") ?? 0
        let baselineMessage = Int64(try database.rows("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM message").first?["maximum"] ?? "0") ?? 0
        let ledgerHome = PrivateReport.directory.appendingPathComponent("messages-command-cleanup", isDirectory: true)
        try FileManager.default.createDirectory(at: ledgerHome, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let ledger = ledgerHome.appendingPathComponent(run + ".json")
        var registration: [String: Any] = ["scenario": "MSG-PARITY-SELFTEST", "run_id": run, "baseline_max_row_id": baselineMessage, "baseline_max_chat_row_id": baseline, "cleanup_state": "pending_native_cleanup"]
        try PrivateReport.write(registration, to: ledger)
        cleanup = "pending_native_cleanup"
        let creations = URL(fileURLWithPath: ledger.path + ".creations.json")
        @Sendable func captureCreatedIDs() throws -> [String] {
            let reader = try MessagesDatabase()
            let ids = try reader.rows("SELECT guid FROM message WHERE ROWID>? AND (instr(text,?)>0 OR instr(attributedBody,CAST(? AS BLOB))>0)", bindings: [String(baselineMessage), tag, tag]).compactMap { $0["guid"] }
            try PrivateReport.write(["run_id": run, "ids": ids], to: creations)
            return ids
        }
        let capture = Task.detached {
            while !Task.isCancelled {
                _ = try? captureCreatedIDs()
                do { try await Task.sleep(for: .milliseconds(100)) } catch { break }
            }
        }
        defer { capture.cancel() }
        let text = tag + " isolated parity proof"
        do {
            guard try !selected.chat.isEmpty || MessagesSelfTestIsolation.unused([handle], database: database) == [handle] else { throw SourceFailure.conflict }
            lateEffectPossible = true
            writesSubmitted += 1
            let result = try await MessagesParityFixture.send(text: text, handle: handle, existingChat: selected.chat)
            guard result["observed"].bool, !result["native_key"].string.isEmpty else {
                prerequisiteReason = "isolated_fixture_send_" + (result["disposition"].string.isEmpty ? "unobserved" : result["disposition"].string)
                throw SourceFailure.capabilityUnproven
            }
            let guid = result["native_key"].string
            let echoDeadline = ContinuousClock.now.advanced(by: .seconds(10))
            var echoObserved = false
            repeat {
                echoObserved = !(try database.rows("SELECT guid FROM message WHERE ROWID>? AND is_from_me=0 AND (instr(text,?)>0 OR instr(attributedBody,CAST(? AS BLOB))>0)", bindings: [String(baselineMessage), tag, tag])).isEmpty
                if echoObserved { break }
                try await Task.sleep(for: .milliseconds(100))
            } while ContinuousClock.now < echoDeadline
            guard echoObserved else {
                prerequisiteReason = "isolated_fixture_echo_unobserved"
                throw SourceFailure.capabilityUnproven
            }
            lateEffectPossible = false
            let chats = try database.rows("SELECT c.guid,c.service_name,c.ROWID AS position FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", bindings: [guid])
            guard chats.count == 1, let chat = chats.first?["guid"], let position = Int64(chats.first?["position"] ?? ""), (selected.chat.isEmpty ? position > baseline : chat == selected.chat) else { throw SourceFailure.conflict }
            let members = try database.rows("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", bindings: [chat])
            guard members.count == 1, members.allSatisfy({ MessagesProbes.normalize($0["id"] ?? "") == handle }) else { throw SourceFailure.conflict }
            registration["created_chat_key"] = chat
            registration["created_by_run"] = selected.chat.isEmpty
            registration["existing_self_chat"] = !selected.chat.isEmpty
            try PrivateReport.write(registration, to: ledger)
            let target = Prepared(nativeState: .object(["service": .string(chats.first?["service_name"] ?? ""), "conversation_target": .bool(true), "message_guid": .string(guid), "target": .object(["text": .string(text), "is_from_me": .bool(true)])]), fingerprint: UUID().uuidString, action: .object(["native_key": .string(chat)]))
            steps = try await MessagesParity.probe(target)
        } catch { blocked(prerequisiteReason ?? "isolated_fixture_" + MessagesParity.failureReason(error)) }
        do {
            var cleaned = false
            for _ in 0..<3 {
                registration["created_message_ids"] = try captureCreatedIDs()
                try PrivateReport.write(registration, to: ledger)
                let data = try await MessagesCleanup.run(["--tag", tag, "--owned-only"])
                let result = try JSONDecoder().decode(JSON.self, from: data)
                try await Task.sleep(for: .milliseconds(500))
                _ = try captureCreatedIDs()
                afterIDs = try activeIDs(database)
                let baselineRestored = try await existingBaseline?.restoreAndVerify() ?? true
                let togglesRestored = steps.filter { $0["restored"] != .null }.allSatisfy { $0["restored"].bool }
                cleaned = baselineRestored && result["passed"].bool && beforeIDs == afterIDs && (try? MessagesPins.read()) == beforePins && togglesRestored
            }
            cleanup = cleaned ? (lateEffectPossible ? "pending_reconciliation" : "verified_clean") : "pending_native_cleanup"
        } catch { cleanup = "pending_native_cleanup" }
        registration["late_effect_possible"] = lateEffectPossible
        registration["cleanup_state"] = cleanup
        try PrivateReport.write(registration, to: ledger)
        return try save()
    }
}
