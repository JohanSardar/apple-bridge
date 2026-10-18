import Foundation
import SourceSupport

public enum MessagesCleanup {
    public static func run() async throws -> Data {
        let own = Set(try MessagesProbes.discoverSelfHandles().handles)
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        func candidates() throws -> [(String, JSON)] {
            let rows = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID) AND (instr(m.text,'[ab-test ')>0 OR instr(m.attributedBody,CAST('[ab-test ' AS BLOB))>0) ORDER BY m.ROWID DESC", [])
            var result: [(String, JSON)] = []
            for key in rows {
                guard let row = try reader.message(key["guid"]), let item = try reader.messageObservations(row).first, item.0 == "messages.message" else { continue }
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["capture_chat_guid"]])
                guard !peers.isEmpty, peers.allSatisfy({ own.contains(MessagesProbes.normalize($0["id"])) }) else { continue }
                let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: item.2))
                guard target["text"].string.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil else { continue }
                result.append((row["guid"], target))
            }
            return result
        }
        let initial = try candidates().count
        var deleted = 0
        var failure = ""
        func progress(_ remaining: Int, _ phase: String) {
            try? PrivateReport.write(["initial_tagged_self_messages": initial, "deleted": deleted, "remaining_tagged_self_messages": remaining, "phase": phase], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-cleanup-progress.json").path))
        }
        progress(initial,"starting")
        for _ in 0..<200 {
            let remaining = try candidates()
            guard let (guid, target) = remaining.first else { break }
            let prepared = Prepared(nativeState: .object(["target": target, "cleanup_authorized": .bool(true)]), fingerprint: "authorized-tagged-self-cleanup", action: .object(["op": .string("messages.delete_message"), "native_key": .string(guid)]))
            var phase = "open"
            do {
                try await MessagesAccessibility.openMessage(guid)
                if try await MessagesAccessibility.acknowledgeDeletedInfo() { try await MessagesAccessibility.openMessage(guid) }
                phase = "validate"
                try await MessagesAccessibility.validate(prepared)
                phase = "delete"
                _ = try await MessagesAccessibility.perform(prepared)
            } catch { failure = phase + ":" + String(describing: error); break }
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            var after = remaining.count
            repeat {
                after = try candidates().count
                if after < remaining.count { break }
                try await Task.sleep(for: .milliseconds(50))
            } while ContinuousClock.now < deadline
            guard after < remaining.count else { failure = "native_deletion_not_observed"; break }
            deleted += remaining.count - after
            progress(after,"deleting")
        }
        let remaining = try candidates().count
        progress(remaining,failure.isEmpty ? "finished" : failure)
        return try JSONSerialization.data(withJSONObject: ["initial_tagged_self_messages": initial, "deleted": deleted, "remaining_tagged_self_messages": remaining, "all_dates": true, "only_tagged_self_messages": true, "failure": failure, "passed": remaining == 0])
    }
}
