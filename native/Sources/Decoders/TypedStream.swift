import Foundation
import CryptoKit

public struct MessagePart: Codable, Sendable, Equatable {
    public var index: Int
    public var kind: String
    public var text: String?
    public var attachmentGUID: String?
    public var mention: String?
    public var utf16Location: Int
    public var utf16Length: Int
}

public struct MessageBody: Codable, Sendable {
    public var parts: [MessagePart]
    public var text: String?
    public var decodeStatus: String
    public var rawBodyDigest: String?
    public var formatting: [MessageFormatting] = []
}

public struct MessageFormatting: Codable, Sendable {
    public var utf16Location: Int
    public var utf16Length: Int
    public var styles: [String]
    public var effect: String?
}

public enum DecodeFailure: Error { case malformed, unsupported, limit }

public enum BodyDigest {
    public static func hash(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}

final class StreamObject {
    var classes: [String] = []
    var groups: [[StreamValue]] = []
    var string: String? {
        guard classes.contains("NSString") else { return nil }
        for group in groups {
            for value in group {
                if case .bytes(let bytes) = value { return String(data: bytes, encoding: .utf8) }
            }
        }
        return nil
    }
}

indirect enum StreamValue {
    case number(Int64), bytes(Data), object(StreamObject), classes([String]), null, values([StreamValue])
    var number: Int? { if case .number(let n) = self { return Int(exactly: n) }; return nil }
    var object: StreamObject? { if case .object(let o) = self { return o }; return nil }
}

final class StreamReader {
    let bytes: [UInt8]
    var position = 0
    var strings: [Data] = []
    var references: [StreamValue] = []
    var depth = 0
    var payloadRanges: [Range<Int>] = []
    var opaqueRanges: [Range<Int>] = []
    var cStringRanges: [Range<Int>] = []
    init(_ data: Data) { bytes = Array(data) }
    func take(_ count: Int) throws -> Data {
        guard count >= 0, count <= bytes.count - position else { throw DecodeFailure.malformed }
        defer { position += count }
        return Data(bytes[position..<position + count])
    }
    func byte() throws -> UInt8 {
        guard position < bytes.count else { throw DecodeFailure.malformed }
        defer { position += 1 }
        return bytes[position]
    }
    func integer(_ initial: UInt8? = nil) throws -> Int64 {
        let tag = try initial ?? byte()
        switch tag {
        case 0x81:
            let b = Array(try take(2))
            return Int64(Int16(bitPattern: UInt16(b[0]) | UInt16(b[1]) << 8))
        case 0x82:
            let b = Array(try take(4))
            return Int64(Int32(bitPattern: UInt32(b[0]) | UInt32(b[1]) << 8 | UInt32(b[2]) << 16 | UInt32(b[3]) << 24))
        case 0x80...0x91: throw DecodeFailure.unsupported
        default: return Int64(Int8(bitPattern: tag))
        }
    }
    func rawString(payload: Bool = false, cString: Bool = false) throws -> Data {
        guard let length = Int(exactly: try integer()), length >= 0 else { throw DecodeFailure.malformed }
        let start = position
        let value = try take(length)
        if payload { payloadRanges.append(start..<position) }
        if cString { cStringRanges.append(start..<position) }
        return value
    }
    func sharedString(cString: Bool = false) throws -> Data? {
        let tag = try byte()
        if tag == 0x85 { return nil }
        if tag == 0x84 {
            let value = try rawString(cString: cString)
            strings.append(value)
            return value
        }
        let index = Int(try integer(tag)) + 110
        guard strings.indices.contains(index) else { throw DecodeFailure.malformed }
        return strings[index]
    }
    func classChain() throws -> [String] {
        let tag = try byte()
        if tag == 0x85 { return [] }
        if tag != 0x84 {
            let index = Int(try integer(tag)) + 110
            guard references.indices.contains(index), case .classes(let names) = references[index] else { throw DecodeFailure.malformed }
            return names
        }
        guard depth < 64 else { throw DecodeFailure.limit }
        depth += 1
        defer { depth -= 1 }
        guard let data = try sharedString(), let name = String(data: data, encoding: .utf8) else { throw DecodeFailure.malformed }
        _ = try integer()
        let slot = references.count
        references.append(.null)
        let names = [name] + (try classChain())
        references[slot] = .classes(names)
        return names
    }
    func object() throws -> StreamValue {
        let tag = try byte()
        if tag == 0x85 { return .null }
        if tag != 0x84 {
            let index = Int(try integer(tag)) + 110
            guard references.indices.contains(index), case .object = references[index] else { throw DecodeFailure.malformed }
            return references[index]
        }
        guard depth < 64, references.count < 100_000 else { throw DecodeFailure.limit }
        depth += 1
        defer { depth -= 1 }
        let value = StreamObject()
        references.append(.object(value))
        value.classes = try classChain()
        while position < bytes.count && bytes[position] != 0x86 {
            value.groups.append(try group())
        }
        guard try byte() == 0x86 else { throw DecodeFailure.malformed }
        return .object(value)
    }
    func group() throws -> [StreamValue] {
        guard let encoding = try sharedString(), let text = String(data: encoding, encoding: .ascii), !text.isEmpty else { throw DecodeFailure.malformed }
        var types = Array(text)
        return try values(&types, closing: nil)
    }
    func values(_ types: inout [Character], closing: Character?) throws -> [StreamValue] {
        guard depth < 64, types.count <= 4096 else { throw DecodeFailure.limit }
        depth += 1
        defer { depth -= 1 }
        var result: [StreamValue] = []
        while !types.isEmpty {
            let type = types.removeFirst()
            if type == closing { return result }
            switch type {
            case "@": result.append(try object())
            case "+": result.append(.bytes(try rawString(payload: true)))
            case "c", "C", "B": result.append(.number(Int64(try byte())))
            case "i", "I", "s", "S", "l", "L", "q", "Q": result.append(.number(try integer()))
            case "f", "d":
                if position < bytes.count && bytes[position] == 0x83 {
                    position += 1
                    let start = position
                    result.append(.bytes(try take(type == "d" ? 8 : 4)))
                    opaqueRanges.append(start..<position)
                } else {
                    let start = position
                    let tag = position < bytes.count ? bytes[position] : 0
                    result.append(.number(try integer()))
                    opaqueRanges.append((tag == 0x81 || tag == 0x82 ? start + 1 : start)..<position)
                }
            case "*":
                let tag = try byte()
                if tag == 0x85 { result.append(.null) }
                else if tag == 0x84 {
                    let value = StreamValue.bytes(try sharedString(cString: true) ?? Data())
                    references.append(value)
                    result.append(value)
                } else {
                    let index = Int(try integer(tag)) + 110
                    guard references.indices.contains(index), case .bytes = references[index] else { throw DecodeFailure.malformed }
                    result.append(references[index])
                }
            case "%", ":": result.append(.bytes(try sharedString() ?? Data()))
            case "{":
                while !types.isEmpty && types.first != "=" { types.removeFirst() }
                guard types.first == "=" else { throw DecodeFailure.unsupported }
                types.removeFirst()
                result.append(.values(try values(&types, closing: "}")))
            case "[":
                var count = ""
                while let next = types.first, next.isNumber { count.append(types.removeFirst()) }
                guard let n = Int(count), n <= 1_000_000, let end = types.firstIndex(of: "]") else { throw DecodeFailure.malformed }
                let element = Array(types[..<end])
                types.removeFirst(end + 1)
                if element == ["c"] || element == ["C"] { let start = position; result.append(.bytes(try take(n))); opaqueRanges.append(start..<position) }
                else {
                    var elements: [StreamValue] = []
                    for _ in 0..<n { var copy = element; elements += try values(&copy, closing: nil) }
                    result.append(.values(elements))
                }
            default: throw DecodeFailure.unsupported
            }
        }
        if closing != nil { throw DecodeFailure.malformed }
        return result
    }
    func read() throws -> StreamObject {
        guard bytes.count <= 16 * 1024 * 1024, try byte() == 4, try byte() == 11,
              try take(11) == Data("streamtyped".utf8), try integer() == 1000 else { throw DecodeFailure.unsupported }
        let root = try group()
        guard position == bytes.count, root.count == 1, let value = root.first?.object else { throw DecodeFailure.malformed }
        return value
    }
}

public enum TypedStream {
    public static func decode(_ data: Data) -> MessageBody {
        do {
            let root = try StreamReader(data).read()
            guard root.classes.contains("NSAttributedString"), let string = root.groups.first?.first?.object?.string else { throw DecodeFailure.unsupported }
            let text = string as NSString
            let units = Array(string.utf16)
            var location = 0
            var parts: [MessagePart] = []
            var formatting: [MessageFormatting] = []
            var index = 0
            var cursor = 1
            var attributeCache: [Int: [String: StreamValue]] = [:]
            while cursor < root.groups.count {
                let range = root.groups[cursor]
                guard range.count == 2, let attributeIndex = range[0].number, let length = range[1].number, length > 0, length <= text.length - location else { throw DecodeFailure.malformed }
                let end = location + length
                if end < units.count && (0xDC00...0xDFFF).contains(units[end]) && end > 0 && (0xD800...0xDBFF).contains(units[end - 1]) { throw DecodeFailure.malformed }
                cursor += 1
                var attributes: [String: StreamValue] = [:]
                if cursor < root.groups.count, let dictionary = root.groups[cursor].first?.object, dictionary.classes.contains("NSDictionary") {
                    attributes = try dictionaryValues(dictionary)
                    cursor += 1
                    attributeCache[attributeIndex] = attributes
                } else if let cached = attributeCache[attributeIndex] { attributes = cached }
                else { throw DecodeFailure.malformed }
                if let explicit = attributes["__kIMMessagePartAttributeName"]?.object?.groups.flatMap({ $0 }).compactMap(\.number).first { index = explicit }
                let attachment = attributes["__kIMFileTransferGUIDAttributeName"]?.object?.string
                let mention = attributes["__kIMMentionConfirmedMention"]?.object?.string ?? attributes["__kIMMentionConfirmedMentionAttributeName"]?.object?.string
                var styles: [String] = []
                for (key, style) in [("__kIMTextBoldAttributeName", "bold"), ("__kIMTextItalicAttributeName", "italic"), ("__kIMTextUnderlineAttributeName", "underline"), ("__kIMTextStrikethroughAttributeName", "strikethrough")] {
                    if let value = attributes[key], scalar(value) != 0 { styles.append(style) }
                }
                let effectNumber = attributes["__kIMTextEffectAttributeName"].map(scalar)
                let effects = [5: "big", 11: "small", 9: "shake", 8: "nod", 12: "explode", 4: "ripple", 6: "bloom", 10: "jitter"]
                let effect = effectNumber.flatMap { effects[$0] }
                if !styles.isEmpty || effect != nil { formatting.append(MessageFormatting(utf16Location: location, utf16Length: length, styles: styles, effect: effect)) }
                let run = text.substring(with: NSRange(location: location, length: length))
                parts.append(MessagePart(index: index, kind: attributes["__kIMEmojiImageAttributeName"] != nil ? "genmoji" : attachment == nil ? "text" : "attachment", text: attachment == nil && attributes["__kIMEmojiImageAttributeName"] == nil ? run : nil, attachmentGUID: attachment, mention: mention, utf16Location: location, utf16Length: length))
                location += length
                index += 1
            }
            guard location == text.length else { throw DecodeFailure.malformed }
            return MessageBody(parts: parts, text: parts.compactMap(\.text).joined(), decodeStatus: "decoded", rawBodyDigest: nil, formatting: formatting)
        } catch {
            return MessageBody(parts: [], text: nil, decodeStatus: "undecoded", rawBodyDigest: BodyDigest.hash(data))
        }
    }
    private static func scalar(_ value: StreamValue) -> Int {
        value.number ?? value.object?.groups.flatMap { $0 }.compactMap(\.number).last ?? 0
    }
    private static func dictionaryValues(_ object: StreamObject) throws -> [String: StreamValue] {
        guard let count = object.groups.first?.first?.number, count >= 0, count <= 100_000 else { throw DecodeFailure.malformed }
        let values = object.groups.dropFirst().flatMap { $0 }
        guard values.count == count * 2 else { throw DecodeFailure.malformed }
        var result: [String: StreamValue] = [:]
        for index in stride(from: 0, to: values.count, by: 2) {
            guard let key = values[index].object?.string else { throw DecodeFailure.malformed }
            result[key] = values[index + 1]
        }
        return result
    }
}

public enum FixtureScrubber {
    public static func scrub(_ data: Data) throws -> Data {
        let reader = StreamReader(data)
        _ = try reader.read()
        var result = data
        let keys: Set<String> = ["__kIMTextBoldAttributeName", "__kIMTextItalicAttributeName", "__kIMTextUnderlineAttributeName", "__kIMTextStrikethroughAttributeName", "__kIMTextEffectAttributeName", "__kIMEmojiImageAttributeName","__kIMMessagePartAttributeName", "__kIMFileTransferGUIDAttributeName", "__kIMMentionConfirmedMention", "__kIMMentionConfirmedMentionAttributeName", "__kIMBaseWritingDirectionAttributeName", "__kIMDataDetectedAttributeName", "__kIMLinkAttributeName"]
        let encodings: Set<String> = ["q", "Q", "i", "I", "s", "S", "c", "C", "l", "L", "f", "d", "B", "@", "^v"]
        let cStrings = reader.cStringRanges.filter { range in
            guard let string = String(data: data.subdata(in: range), encoding: .utf8) else { return true }
            return !encodings.contains(string)
        }
        for range in reader.opaqueRanges { result.replaceSubrange(range, with: repeatElement(UInt8(0), count: range.count)) }
        for range in reader.payloadRanges + cStrings {
            guard let string = String(data: data.subdata(in: range), encoding: .utf8) else { throw DecodeFailure.malformed }
            if keys.contains(string) { continue }
            let replacement = String(String.UnicodeScalarView(string.unicodeScalars.map { scalar in
                switch scalar.utf8.count {
                case 1: return UnicodeScalar(120)!
                case 2: return UnicodeScalar(0x00A1)!
                case 3: return UnicodeScalar(0x25A1)!
                default: return UnicodeScalar(0x1F7E5)!
                }
            }))
            guard replacement.utf8.count == range.count, replacement.utf16.count == string.utf16.count else { throw DecodeFailure.malformed }
            result.replaceSubrange(range, with: replacement.utf8)
        }
        _ = try StreamReader(result).read()
        return result
    }
}
