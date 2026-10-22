import Foundation

public enum MessagesSelfTestIsolation {
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

    public static func unused(_ own: [String], database: MessagesDatabase) throws -> [String] {
        let participants = try database.rows("SELECT DISTINCT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id").compactMap { $0["id"] }
        return unused(own, participants: participants)
    }
}
