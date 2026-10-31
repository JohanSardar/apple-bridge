import Foundation
import ApplicationServices
import SourceSupport
import Decoders

public struct MessagesSelfTestReadState: Sendable, Codable {
    public let unreadIDs: Set<String>
    public let unreadCount: Int

    public init(unreadIDs: Set<String>, unreadCount: Int) {
        self.unreadIDs = unreadIDs
        self.unreadCount = unreadCount
    }

    public func matches(_ current: MessagesSelfTestReadState) -> Bool {
        if !unreadIDs.isEmpty && unreadIDs == current.unreadIDs { return true }
        return unreadCount == current.unreadCount && (unreadIDs == current.unreadIDs || unreadCount > 0)
    }

    static func savedURL(_ chat: String) -> URL {
        PrivateReport.directory.appendingPathComponent("messages-unread-restore/" + BodyDigest.hash(Data(chat.utf8)) + ".json")
    }

    static func baseline(_ chat: String, through maximum: Int64) throws -> (MessagesSelfTestReadState, Int64) {
        let url = savedURL(chat)
        if FileManager.default.fileExists(atPath: url.path) {
            let saved = try JSONDecoder().decode(Saved.self, from: Data(contentsOf: url))
            guard saved.chat == chat, saved.through <= maximum else { throw SourceFailure.conflict }
            return (saved.state, saved.through)
        }
        let state = try capture(chat, through: maximum)
        try state.save(chat, through: maximum)
        return (state, maximum)
    }

    private struct Saved: Codable {
        let chat: String
        let through: Int64
        let state: MessagesSelfTestReadState
    }

    func save(_ chat: String, through baseline: Int64) throws {
        let url = Self.savedURL(chat)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let data = try JSONEncoder().encode(Saved(chat: chat, through: baseline, state: self))
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    static func complete(_ chat: String) throws {
        let url = savedURL(chat)
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }

    static func capture(_ chat: String, through baseline: Int64) throws -> MessagesSelfTestReadState {
        let reader = try MessagesSelfTestBaseline.reader()
        let rows = try reader.query("SELECT m.guid,m.is_read,m.date,c.last_read_message_timestamp FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND m.ROWID<=? AND m.is_from_me=0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)", [chat, String(baseline)])
        let unreadIDs = Set(rows.filter { $0.integer("is_read") == 0 }.map { $0["guid"] })
        let count = rows.filter { $0.integer("date") > $0.integer("last_read_message_timestamp") }.count
        return MessagesSelfTestReadState(unreadIDs: unreadIDs, unreadCount: count)
    }

    @UIActor
    func restore(_ prepared: Prepared, through baseline: Int64) throws {
        let chat = prepared.action["native_key"].string
        func verified() throws -> Bool {
            let current = try Self.capture(chat, through: baseline)
            let restored = matches(current)
            try PrivateReport.write(["before_unread_message_count": unreadIDs.count, "after_unread_message_count": current.unreadIDs.count, "before_chat_unread_count": unreadCount, "after_chat_unread_count": current.unreadCount, "unread_identities_restored": unreadIDs == current.unreadIDs, "passed": restored], to: PrivateReport.directory.appendingPathComponent("messages-unread-restore-result.json"))
            return restored
        }
        if try verified() { return }
        guard unreadCount > 0 || !unreadIDs.isEmpty else { throw MessagesSelfTestBaselineFailure.unreadRestoreFailed }
        let (session, items) = try MessagesParity.conversationMenu(prepared)
        let targets = items.filter { MessagesAccessibility.value($0, kAXTitleAttribute) == "Mark as Unread" }
        guard targets.count == 1 else { try? MessagesParity.dismiss(session); throw MessagesUIFailure.menuUnavailable }
        try session.fence()
        try MessagesVariantUI.press(targets[0])
        let deadline = Date().addingTimeInterval(5)
        repeat {
            if try verified() { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        throw MessagesSelfTestBaselineFailure.unreadRestoreFailed
    }
}
