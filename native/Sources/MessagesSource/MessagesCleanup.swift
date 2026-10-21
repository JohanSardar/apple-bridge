import Foundation
import SourceSupport

public enum MessagesCleanup {
    public static func run(_ arguments: [String]) async throws -> Data {
        var tag = ""
        if let index = arguments.firstIndex(of: "--tag") {
            guard index + 1 < arguments.count,
                  arguments[index + 1].range(of: #"^\[ab-test [A-Za-z0-9_-]+\]$"#, options: .regularExpression) != nil else { throw ProbeFailure.invalidArguments }
            tag = arguments[index + 1]
        }
        var approved: Set<String> = []
        var groupHandles: [String: String] = [:]
        if arguments.contains("--approved-contacts") {
            let file = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/apple-bridge/test-contacts.env")
            for line in try String(contentsOf: file, encoding: .utf8).components(separatedBy: .newlines) {
                let parts = line.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "export ", with: "").split(separator: "=", maxSplits: 1).map(String.init)
                if parts.count == 2, ["REMY_IMESSAGE_TEST_HANDLE", "REMY_IMESSAGE_TEST_EMAIL", "BEN_GROUP_TEST_HANDLE"].contains(parts[0].trimmingCharacters(in: .whitespaces)) {
                    let handle = MessagesProbes.normalize(parts[1].trimmingCharacters(in: CharacterSet(charactersIn: " \"'")))
                    if !handle.isEmpty { approved.insert(handle); groupHandles[parts[0].trimmingCharacters(in: .whitespaces)] = handle }
                }
            }
        }
        var draftsCleared = try await MessagesAccessibility.clearTaggedDraft(tag) ? 1 : 0
        if arguments.contains("--group-draft") {
            guard let remy = groupHandles["REMY_IMESSAGE_TEST_HANDLE"], let ben = groupHandles["BEN_GROUP_TEST_HANDLE"], remy != ben else { throw ProbeFailure.invalidArguments }
            if try await MessagesAccessibility.clearTaggedDraft(tag, groupHandles: [remy, ben]) { draftsCleared += 1 }
        }
        let own = Set(try MessagesProbes.discoverSelfHandles().handles)
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        var mediaNames: Set<String> = []
        var mediaBaseline: Int64 = 0
        if !tag.isEmpty {
            let run = String(tag.dropFirst(9).dropLast())
            let ledger = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/messages-command-cleanup/" + run + ".json")
            if let bytes = try? Data(contentsOf: ledger), let registration = try? JSONDecoder().decode(JSON.self, from: bytes), registration["scenario"].string == "WEB-MEDIA-VARIANTS", registration["run_id"].string == run {
                mediaBaseline = Int64(registration["baseline_max_row_id"].number)
                mediaNames = Set(registration["media_attachment_names"].array.map(\.string).filter { $0.hasPrefix("ab-" + run + ".") && $0 == URL(fileURLWithPath: $0).lastPathComponent })
            }
        }
        func candidates() throws -> [(String, JSON)] {
            var rows = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID) AND (instr(m.text,'[ab-test ')>0 OR instr(m.attributedBody,CAST('[ab-test ' AS BLOB))>0) ORDER BY m.ROWID DESC", [])
            var mediaByMessage: [String: [String]] = [:]
            if !mediaNames.isEmpty, mediaBaseline > 0 {
                for row in try reader.query("SELECT m.guid,a.filename FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id JOIN chat_message_join c ON c.message_id=m.ROWID WHERE m.ROWID>? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", [String(mediaBaseline)]) {
                    let name = URL(fileURLWithPath: row["filename"]).lastPathComponent
                    if mediaNames.contains(name) { mediaByMessage[row["guid"], default: []].append(name) }
                }
                rows += try reader.query("SELECT guid FROM message WHERE ROWID>?", [String(mediaBaseline)]).filter { mediaByMessage[$0["guid"]] != nil && !rows.map({ $0["guid"] }).contains($0["guid"]) }
            }
            var result: [(String, JSON)] = []
            for key in rows {
                guard let row = try reader.message(key["guid"]), let item = try reader.messageObservations(row).first, item.0 == "messages.message" else { continue }
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["capture_chat_guid"]])
                guard !peers.isEmpty, peers.allSatisfy({ own.union(approved).contains(MessagesProbes.normalize($0["id"])) }) else { continue }
                var fields = item.2
                if let names = mediaByMessage[row["guid"]], peers.allSatisfy({ own.contains(MessagesProbes.normalize($0["id"])) }) {
                    fields["cleanup_media_names"] = names
                    let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: fields))
                    result.append((row["guid"], target))
                    continue
                }
                let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: fields))
                guard target["text"].string.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil else { continue }
                guard tag.isEmpty || target["text"].string.contains(tag) else { continue }
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
                try await MessagesAccessibility.openMessage(guid, settle: 0.25)
                if try await MessagesAccessibility.acknowledgeDeletedInfo() { try await MessagesAccessibility.openMessage(guid, settle: 0.25) }
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
        return try JSONSerialization.data(withJSONObject: ["initial_tagged_self_messages": initial, "deleted": deleted, "remaining_tagged_self_messages": remaining, "all_dates": true, "drafts_cleared": draftsCleared, "only_tagged_self_messages": approved.isEmpty, "approved_contacts_included": !approved.isEmpty, "tag_filtered": !tag.isEmpty, "failure": failure, "passed": remaining == 0])
    }
}
