import Foundation

public enum MessagesSelfTestIsolation {
    public static func matchesConversation(chat: String, memberships: [String], expected: String, visibleTexts: [String]) -> Bool {
        !chat.isEmpty && memberships == [chat] && !expected.isEmpty && visibleTexts.contains(expected)
    }
    private static func identity(_ raw: String) -> (email: Bool, value: String)? {
        var value = (raw.removingPercentEncoding ?? raw).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let prefixes = ["imessage:", "sms:", "rcs:", "mailto:", "tel:", "p:", "e:", "imessage;-;", "sms;-;", "rcs;-;"]
        while let prefix = prefixes.first(where: { value.hasPrefix($0) }) {
            value.removeFirst(prefix.count)
            value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if value.contains("@") {
            let parts = value.split(separator: "@", omittingEmptySubsequences: false)
            guard parts.count == 2, parts.allSatisfy({ !$0.isEmpty }), !value.contains(where: { $0.isWhitespace }) else { return nil }
            return (true, value.precomposedStringWithCanonicalMapping)
        }
        let allowed = CharacterSet(charactersIn: "+0123456789()-. ").union(.whitespacesAndNewlines)
        guard value.unicodeScalars.allSatisfy({ allowed.contains($0) }) else { return nil }
        var digits = value.filter { $0.isASCII && $0.isNumber }
        if digits.hasPrefix("00") { digits.removeFirst(2) }
        while digits.hasPrefix("0") { digits.removeFirst() }
        guard digits.count >= 7 else { return nil }
        return (false, digits)
    }

    public static func unused(_ own: [String], participants: [String]) -> [String] {
        let known = participants.compactMap(identity)
        return own.filter { alias in
            guard let candidate = identity(alias) else { return false }
            return !known.contains { peer in
                guard peer.email == candidate.email else { return false }
                return candidate.email ? peer.value == candidate.value : peer.value.hasSuffix(candidate.value) || candidate.value.hasSuffix(peer.value)
            }
        }
    }

    public static func isSelfChat(_ participants: [String], own: [String]) -> Bool {
        guard participants.count == 1, let peer = identity(participants[0]) else { return false }
        return own.compactMap(identity).contains { $0.email == peer.email && $0.value == peer.value }
    }

    public static func ownsMessage(rowID: Int64, text: String, tag: String, baseline: Int64) -> Bool {
        tag.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]$"#, options: .regularExpression) != nil && rowID > baseline && text.hasPrefix(tag + " ")
    }

    public static func ownsDeletedSelection(_ deleted: [String], created: [String]) -> Bool {
        !created.isEmpty && Set(deleted).count == deleted.count && Set(created).count == created.count && Set(deleted) == Set(created)
    }

    public static func accountHandles(database: MessagesDatabase) throws -> [String] {
        let columns = Set(try database.rows("PRAGMA table_info(message)").compactMap { $0["name"] })
        var handles: Set<String> = []
        for column in ["destination_caller_id", "account_login"] where columns.contains(column) {
            for row in try database.rows("SELECT DISTINCT \(column) AS handle FROM message WHERE is_from_me=1 AND \(column) IS NOT NULL") {
                if let raw = row["handle"], identity(raw) != nil { handles.insert(MessagesProbes.normalize(raw)) }
            }
        }
        return handles.sorted()
    }

    public static func target(database: MessagesDatabase) throws -> (handle: String, chat: String)? {
        let own = try accountHandles(database: database)
        if let handle = try unused(own, database: database).first { return (handle, "") }
        let pins = try MessagesPins.read()
        let chats = try database.rows("SELECT c.guid,c.chat_identifier FROM chat c WHERE c.service_name='iMessage' AND EXISTS(SELECT 1 FROM chat_message_join j WHERE j.chat_id=c.ROWID) ORDER BY c.ROWID DESC")
        for chat in chats {
            guard let guid = chat["guid"], let identifier = chat["chat_identifier"], !pins.contains(identifier) || pins.last == identifier else { continue }
            let peers = try database.rows("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", bindings: [guid]).map { $0["id"] ?? "" }
            guard isSelfChat(peers, own: own), let peer = peers.first,
                  let handle = own.first(where: { isSelfChat([peer], own: [$0]) }) else { continue }
            return (handle, guid)
        }
        return nil
    }

    public static func unused(_ own: [String], database: MessagesDatabase) throws -> [String] {
        let participants = try database.rows("SELECT DISTINCT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID LEFT JOIN handle h ON h.ROWID=j.handle_id").compactMap { $0["id"] }
        return unused(own, participants: participants)
    }
}
