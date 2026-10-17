import Foundation

public struct MessageReaction: Codable, Sendable, Equatable {
    public let messageGUID: String
    public let partIndex: Int
    public let kind: String
    public let emoji: String?
    public let removed: Bool
    public static func decode(type: Int, target: String, emoji: String?) -> MessageReaction? {
        let removed = (3000...3007).contains(type)
        let index = type - (removed ? 3000 : 2000)
        guard (0...7).contains(index) else { return nil }
        var guid = target
        var part = 0
        if target.hasPrefix("p:") {
            guard let slash = target.firstIndex(of: "/"), let value = Int(target[target.index(target.startIndex, offsetBy: 2)..<slash]), value >= 0 else { return nil }
            part = value
            guid = String(target[target.index(after: slash)...])
        } else if target.hasPrefix("bp:") { guid = String(target.dropFirst(3)) }
        guard !guid.isEmpty else { return nil }
        return MessageReaction(messageGUID: guid, partIndex: part, kind: ["love", "like", "dislike", "laugh", "emphasize", "question", "emoji", "sticker"][index], emoji: emoji, removed: removed)
    }
}

public struct MessageEdit: Codable, Sendable {
    public let partIndex: Int
    public let timestamp: Double?
    public let body: MessageBody
}

public struct MessageSummary: Codable, Sendable {
    public let editHistory: [MessageEdit]
    public let retractedParts: [Int]
    public let decodeStatus: String
    public let rawBodyDigest: String?
    public static func decode(_ data: Data) -> MessageSummary {
        do {
            guard let root = try PropertyListSerialization.propertyList(from: data, options: [], format: nil) as? [String: Any] else { throw DecodeFailure.malformed }
            var edits: [MessageEdit] = []
            if let field = root["ec"] {
                guard let histories = field as? [String: Any] else { throw DecodeFailure.malformed }
                for key in histories.keys.sorted() {
                    guard let part = Int(key), part >= 0, let history = histories[key] as? [[String: Any]] else { throw DecodeFailure.malformed }
                    for edit in history {
                        guard let bytes = edit["t"] as? Data else { throw DecodeFailure.malformed }
                        edits.append(MessageEdit(partIndex: part, timestamp: (edit["d"] as? NSNumber)?.doubleValue, body: TypedStream.decode(bytes)))
                    }
                }
            }
            var retracted: [Int] = []
            if let parts = root["rp"] as? [NSNumber] { retracted = parts.map(\.intValue) }
            else if let parts = root["rp"] as? [String: Any] {
                retracted = try parts.keys.map { guard let n = Int($0), n >= 0 else { throw DecodeFailure.malformed }; return n }
            } else if root["rp"] != nil { throw DecodeFailure.malformed }
            let known = Set(root.keys).isSubset(of: ["ec", "rp", "otr", "ust", "amc", "ams", "ep"])
            let complete = known && edits.allSatisfy { $0.body.decodeStatus == "decoded" }
            return MessageSummary(editHistory: edits, retractedParts: retracted.sorted(), decodeStatus: complete ? "decoded" : "partial", rawBodyDigest: complete ? nil : BodyDigest.hash(data))
        } catch {
            return MessageSummary(editHistory: [], retractedParts: [], decodeStatus: "undecoded", rawBodyDigest: BodyDigest.hash(data))
        }
    }
}

extension FixtureScrubber {
    public static func summary(_ data: Data) throws -> Data {
        let root = try PropertyListSerialization.propertyList(from: data, options: [], format: nil)
        func scrubValue(_ value: Any) throws -> Any {
            if let bytes = value as? Data { return try scrub(bytes) }
            if value is String { return "synthetic" }
            if let values = value as? [Any] { return try values.map(scrubValue) }
            if let dictionary = value as? [String: Any] {
                let safe: Set<String> = ["amc", "d", "ec", "ep", "le", "lo", "otr", "t", "ust", "rp", "ams"]
                var result: [String: Any] = [:]
                for (offset, key) in dictionary.keys.sorted().enumerated() {
                    let name = Int(key) != nil || safe.contains(key) ? key : "synthetic_key_\(offset)"
                    result[name] = key == "d" ? 123.5 : try scrubValue(dictionary[key]!)
                }
                return result
            }
            return value
        }
        return try PropertyListSerialization.data(fromPropertyList: scrubValue(root), format: .binary, options: 0)
    }
}
