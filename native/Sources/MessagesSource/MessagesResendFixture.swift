#if LAB
import Foundation
import SourceSupport

@MainActor
public enum MessagesResendFixture {
    public static func run(_ run: String) async throws -> JSON {
        guard run.range(of: #"^[A-Za-z0-9_-]+$"#, options: .regularExpression) != nil else { throw SourceFailure.invalid }
        let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + run + ".json")
        var registration = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: ledger))
        guard registration["run_id"].string == run, registration["scenario"].string == "MSG-RESEND",
              registration["baseline_max_row_id"].number > 0,
              let target = try MessagesSelfTestIsolation.target(database: MessagesDatabase()), !target.chat.isEmpty else { throw SourceFailure.invalid }
        let reader = try MessagesReader(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path)
        guard try reader.query("SELECT 1 FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.is_from_me=0 AND m.is_read=0 AND COALESCE(m.associated_message_type,0)=0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) LIMIT 1", [target.chat]).isEmpty else { throw MessagesSelfTestBaselineFailure.unreadMessages }
        let tag = "[ab-test " + run + "]"
        let text = tag + " failed fixture"
        let native = try MessagesNativeSession.failedSelfFixture(text, chat: target.chat) { guid in
            var fields = registration.object
            fields["created_message_ids"] = .array(registration["created_message_ids"].array + [.string(guid)])
            registration = .object(fields)
            try JSONEncoder().encode(registration).write(to: ledger, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: ledger.path)
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(30))
        repeat {
            let rows = try reader.query("SELECT m.guid,m.error FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND c.guid=? AND m.is_from_me=1 AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)", [String(Int64(registration["baseline_max_row_id"].number)), target.chat, tag, tag])
            if rows.count == 1, rows[0]["guid"] == native, rows[0].integer("error") != 0 { return .object(["passed": .bool(true), "failed_fixture_count": .number(1)]) }
            guard rows.count <= 1 else { break }
            try await Task.sleep(for: .milliseconds(100))
        } while ContinuousClock.now < deadline
        return .object(["passed": .bool(false), "failed_fixture_count": .number(0), "owned_send_cancel_requested": .bool(true)])
    }
}
#endif
