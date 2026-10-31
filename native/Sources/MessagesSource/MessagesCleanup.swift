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
        let scopedChat: Int64?
        if let index = arguments.firstIndex(of: "--only-chat-id") {
            guard index + 1 < arguments.count, let value = Int64(arguments[index + 1]), value > 0 else { throw ProbeFailure.invalidArguments }
            scopedChat = value
        } else {
            scopedChat = nil
        }
        guard scopedChat == nil || !tag.isEmpty else { throw ProbeFailure.invalidArguments }
        let scopedGUID: String?
        if let index = arguments.firstIndex(of: "--only-guid") {
            guard scopedChat != nil, index + 1 < arguments.count, !arguments[index + 1].isEmpty, arguments[index + 1].count <= 256, !arguments[index + 1].contains("\n") else { throw ProbeFailure.invalidArguments }
            scopedGUID = arguments[index + 1]
        } else {
            scopedGUID = nil
        }
        guard !(arguments.contains("--skip-draft") && arguments.contains("--group-draft")) else { throw ProbeFailure.invalidArguments }
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
        var draftsCleared = arguments.contains("--skip-draft") ? 0 : (try await MessagesAccessibility.clearTaggedDraft(tag) ? 1 : 0)
        if arguments.contains("--group-draft") {
            guard let remy = groupHandles["REMY_IMESSAGE_TEST_HANDLE"], let ben = groupHandles["BEN_GROUP_TEST_HANDLE"], remy != ben else { throw ProbeFailure.invalidArguments }
            if try await MessagesAccessibility.clearTaggedDraft(tag, groupHandles: [remy, ben]) { draftsCleared += 1 }
        }
        let own = Set(try MessagesProbes.discoverSelfHandles().handles)
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        var ownedIDs: Set<String> = []
        var ownedBaseline: Int64?
        let ownedOnly = arguments.contains("--owned-only")
        var equivalentOwnedMedia = false
        var mediaProof: MessagesMediaCleanupProof?
        var interactiveText = ""
        var interactiveBaseline: Int64 = 0
        if !tag.isEmpty {
            let run = String(tag.dropFirst(9).dropLast())
            let ledger = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/messages-command-cleanup/" + run + ".json")
            if let bytes = try? Data(contentsOf: ledger), let registration = try? JSONDecoder().decode(JSON.self, from: bytes) {
                mediaProof = try? MessagesMediaCleanupProof(registration: registration, tag: tag)
                if registration["run_id"].string == run {
                    ownedBaseline = Int64(registration["baseline_max_row_id"].number)
                    ownedIDs = Set(registration["created_message_ids"].array.map { $0.string }.filter { !$0.isEmpty })
                    for name in ["native_key", "incoming_echo_native_key"] where !registration[name].string.isEmpty { ownedIDs.insert(registration[name].string) }
                    for part in registration["native_parts"].array where !part["native_key"].string.isEmpty { ownedIDs.insert(part["native_key"].string) }
                }
            }
            if let bytes = try? Data(contentsOf: ledger), let registration = try? JSONDecoder().decode(JSON.self, from: bytes), registration["scenario"].string == "MSG-INTERACTIVE-GATE", registration["run_id"].string == run, registration["interactive_text"].string == "Apple Bridge interactive self-check " + run {
                interactiveText = registration["interactive_text"].string
                interactiveBaseline = Int64(registration["baseline_max_row_id"].number)
            }
        }
        guard !ownedOnly || (!tag.isEmpty && ownedBaseline != nil && approved.isEmpty) else { throw ProbeFailure.invalidArguments }
        func candidates() throws -> [(String, JSON)] {
            if ownedOnly {
                let run = String(tag.dropFirst(9).dropLast())
                let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/messages-command-cleanup/" + run + ".json.creations.json")
                if let bytes = try? Data(contentsOf: path), let report = try? JSONDecoder().decode(JSON.self, from: bytes), report["run_id"].string == run {
                    ownedIDs.formUnion(report["ids"].array.map { $0.string }.filter { !$0.isEmpty })
                }
            }
            var rows = try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID) AND (instr(m.text,'[ab-test ')>0 OR instr(m.attributedBody,CAST('[ab-test ' AS BLOB))>0) ORDER BY m.ROWID DESC", [])
            if ownedOnly, let baseline = ownedBaseline, !ownedIDs.isEmpty {
                let known = Set(rows.map { $0["guid"] })
                rows += try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join c ON c.message_id=m.ROWID WHERE m.ROWID>? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", [String(baseline)]).filter { ownedIDs.contains($0["guid"]) && !known.contains($0["guid"]) }
            }
            var mediaByMessage: [String: [String]] = [:]
            if !interactiveText.isEmpty, interactiveBaseline > 0 {
                rows += try reader.query("SELECT DISTINCT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.ROWID>? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=j.chat_id) AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(interactiveBaseline), interactiveText, interactiveText])
            }
            if let proof = mediaProof {
                let historical = try reader.query("SELECT a.filename,a.transfer_name FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID WHERE j.message_id<=?", [String(proof.baseline)])
                equivalentOwnedMedia = !historical.contains { row in
                    proof.names.contains(URL(fileURLWithPath: row["filename"]).lastPathComponent) || proof.names.contains(row["transfer_name"])
                }
                guard equivalentOwnedMedia else { throw SourceFailure.conflict }
                var attachmentNames: [String: [String]] = [:]
                var positions: [String: Int64] = [:]
                for row in try reader.query("SELECT m.guid,m.ROWID AS native_row_id,a.filename,a.transfer_name FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id JOIN chat_message_join c ON c.message_id=m.ROWID WHERE m.ROWID>? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", [String(proof.baseline)]) {
                    attachmentNames[row["guid"], default: []].append(row["transfer_name"].isEmpty ? row["filename"] : row["transfer_name"])
                    positions[row["guid"]] = row.integer("native_row_id")
                }
                for (guid, names) in attachmentNames {
                    guard try reader.message(guid) != nil else { continue }
                    let peers = try reader.query("SELECT DISTINCT h.id FROM message m JOIN chat_message_join cm ON cm.message_id=m.ROWID JOIN chat c ON c.ROWID=cm.chat_id JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE m.guid=? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.ROWID)", [guid]).map { $0["id"] }
                    if proof.authorizes(rowID: positions[guid] ?? 0, attachments: names, peers: peers, own: own) { mediaByMessage[guid] = proof.targetNames(names) }
                }
                rows += try reader.query("SELECT guid FROM message WHERE ROWID>?", [String(proof.baseline)]).filter { mediaByMessage[$0["guid"]] != nil && !rows.map({ $0["guid"] }).contains($0["guid"]) }
            }
            var result: [(String, JSON)] = []
            for key in rows {
                if ownedOnly {
                    guard (ownedIDs.contains(key["guid"]) || mediaByMessage[key["guid"]] != nil), let baseline = ownedBaseline,
                          let position = try reader.query("SELECT ROWID AS position FROM message WHERE guid=?", [key["guid"]]).first,
                          position.integer("position") > baseline else { continue }
                }
                if let scopedGUID, key["guid"] != scopedGUID { continue }
                if let scopedChat, try reader.query("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID WHERE m.guid=? AND j.chat_id=? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID)", [key["guid"], String(scopedChat)]).isEmpty { continue }
                guard let row = try reader.message(key["guid"]), let item = try reader.messageObservations(row).first, item.0 == "messages.message" else { continue }
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["capture_chat_guid"]])
                guard !peers.isEmpty, peers.allSatisfy({ (ownedOnly ? own : own.union(approved)).contains(MessagesProbes.normalize($0["id"])) }) else { continue }
                if ownedOnly {
                    let allPeers = try reader.query("SELECT DISTINCT h.id FROM message m JOIN chat_message_join cm ON cm.message_id=m.ROWID JOIN chat_handle_join j ON j.chat_id=cm.chat_id JOIN handle h ON h.ROWID=j.handle_id WHERE m.guid=?", [key["guid"]])
                    guard !allPeers.isEmpty, allPeers.allSatisfy({ own.contains(MessagesProbes.normalize($0["id"])) }) else { continue }
                }
                var fields = item.2
                if let names = mediaByMessage[row["guid"]], peers.allSatisfy({ own.contains(MessagesProbes.normalize($0["id"])) }) {
                    fields["cleanup_media_names"] = names
                    fields["cleanup_self_echo"] = ownedOnly
                    fields["cleanup_equivalent_owned_media"] = ownedOnly && equivalentOwnedMedia
                    let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: fields))
                    result.append((row["guid"], target))
                    continue
                }
                let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: fields))
                if !interactiveText.isEmpty, target["text"].string == interactiveText, peers.allSatisfy({ own.contains(MessagesProbes.normalize($0["id"])) }) {
                    result.append((row["guid"], target))
                    continue
                }
                guard target["text"].string.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil else {
                    if ownedOnly, row.integer("date_retracted") == 0 { throw SourceFailure.conflict }
                    continue
                }
                guard tag.isEmpty || target["text"].string.contains(tag) else {
                    if ownedOnly { throw SourceFailure.conflict }
                    continue
                }
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
                try await MessagesAccessibility.openMessage(guid, settle: ownedOnly ? 1 : 0.25, resetEditor: scopedChat == nil)
                if scopedChat == nil, try await MessagesAccessibility.acknowledgeDeletedInfo() { try await MessagesAccessibility.openMessage(guid, settle: 0.25) }
                phase = "validate"
                try await MessagesAccessibility.validate(prepared)
                phase = "delete"
                _ = try await MessagesAccessibility.perform(prepared)
            } catch { failure = phase + ":" + String(describing: error); break }
            let deadline = ContinuousClock.now.advanced(by: .seconds(ownedOnly ? 10 : 2))
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
