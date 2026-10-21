import Foundation

struct ArchiveUID { let index: Int }

final class ArchivePlist {
    let bytes: [UInt8]
    let offsets: [Int]
    let referenceSize: Int
    let top: Int
    var remaining = 200_000
    init(_ data: Data) throws {
        let bytes = Array(data)
        self.bytes = bytes
        guard bytes.count >= 40, bytes.count <= 16 * 1024 * 1024, bytes.starts(with: Array("bplist00".utf8)) else { throw DecodeFailure.malformed }
        func number(_ start: Int, _ size: Int) throws -> Int {
            guard start >= 0, size > 0, size <= 8, start <= bytes.count - size else { throw DecodeFailure.malformed }
            var n: UInt64 = 0
            for b in bytes[start..<start + size] { n = n << 8 | UInt64(b) }
            guard n <= UInt64(Int.max) else { throw DecodeFailure.limit }
            return Int(n)
        }
        let trailer = bytes.count - 32
        let size = Int(bytes[trailer + 6])
        referenceSize = Int(bytes[trailer + 7])
        let count = try number(trailer + 8, 8)
        top = try number(trailer + 16, 8)
        let start = try number(trailer + 24, 8)
        guard count > 0, count <= 100_000, top < count, referenceSize > 0, referenceSize <= 8, size > 0, size <= 8, start >= 8, start <= trailer, count <= (trailer - start) / size else { throw DecodeFailure.limit }
        offsets = try (0..<count).map { try number(start + $0 * size, size) }
    }
    func integer(_ start: Int, _ size: Int) throws -> UInt64 {
        guard start >= 8, size > 0, size <= 8, start <= bytes.count - 32 - size else { throw DecodeFailure.malformed }
        return bytes[start..<start + size].reduce(UInt64(0)) { $0 << 8 | UInt64($1) }
    }
    func object(_ index: Int, depth: Int = 0) throws -> Any {
        remaining -= 1
        guard offsets.indices.contains(index), depth < 64, remaining > 0 else { throw DecodeFailure.limit }
        let start = offsets[index]
        guard start >= 8, start < bytes.count - 32 else { throw DecodeFailure.malformed }
        let tag = bytes[start]
        let kind = tag >> 4
        var count = Int(tag & 15)
        var body = start + 1
        if count == 15 && [4, 5, 6, 10, 13].contains(kind) {
            guard body < bytes.count - 32, bytes[body] >> 4 == 1, bytes[body] & 15 <= 3 else { throw DecodeFailure.malformed }
            let length = 1 << Int(bytes[body] & 15)
            let n = try integer(body + 1, length)
            guard n <= 100_000 else { throw DecodeFailure.limit }
            count = Int(n)
            body += 1 + length
        }
        func slice(_ n: Int) throws -> Data {
            guard n >= 0, body <= bytes.count - 32 - n else { throw DecodeFailure.malformed }
            return Data(bytes[body..<body + n])
        }
        func child(_ offset: Int) throws -> Any {
            let ref = try integer(body + offset * referenceSize, referenceSize)
            guard ref < UInt64(offsets.count) else { throw DecodeFailure.malformed }
            return try object(Int(ref), depth: depth + 1)
        }
        switch kind {
        case 0:
            if count == 8 { return false }
            if count == 9 { return true }
            return NSNull()
        case 1:
            guard count <= 3 else { throw DecodeFailure.limit }
            let bits = try integer(body, 1 << count)
            return NSNumber(value: Int64(bitPattern: bits))
        case 2:
            if count == 2 { return Double(Float(bitPattern: UInt32(try integer(body, 4)))) }
            guard count == 3 else { throw DecodeFailure.malformed }
            return Double(bitPattern: try integer(body, 8))
        case 3: return Date(timeIntervalSinceReferenceDate: Double(bitPattern: try integer(body, 8)))
        case 4: return try slice(count)
        case 5, 6:
            guard let value = String(data: try slice(count * (kind == 6 ? 2 : 1)), encoding: kind == 6 ? .utf16BigEndian : .ascii) else { throw DecodeFailure.malformed }
            return value
        case 8:
            let n = try integer(body, count + 1)
            guard n <= UInt64(Int.max) else { throw DecodeFailure.limit }
            return ArchiveUID(index: Int(n))
        case 10: return try (0..<count).map { try child($0) }
        case 13:
            var value: [String: Any] = [:]
            for i in 0..<count {
                guard let key = try child(i) as? String else { throw DecodeFailure.malformed }
                value[key] = try child(i + count)
            }
            return value
        default: throw DecodeFailure.unsupported
        }
    }
}

public enum MessageArchive {
    public static func decode(_ data: Data) -> [String: Any]? {
        do {
            let raw: Any
            if data.starts(with: Data("bplist00".utf8)) {
                let reader = try ArchivePlist(data)
                raw = try reader.object(reader.top)
            } else {
                guard data.count <= 16 * 1024 * 1024 else { return nil }
                raw = try PropertyListSerialization.propertyList(from: data, format: nil)
            }
            guard let archive = raw as? [String: Any] else { return nil }
            guard let objects = archive["$objects"] as? [Any], let top = archive["$top"] as? [String: Any] else { return archive }
            var remaining = 100_000
            func resolve(_ value: Any, _ visited: Set<Int>, _ depth: Int) throws -> Any {
                remaining -= 1
                guard depth < 64, remaining > 0 else { throw DecodeFailure.limit }
                if let uid = value as? ArchiveUID {
                    if uid.index == 0 { return NSNull() }
                    guard objects.indices.contains(uid.index), !visited.contains(uid.index) else { throw DecodeFailure.malformed }
                    return try resolve(objects[uid.index], visited.union([uid.index]), depth + 1)
                }
                if let array = value as? [Any] { return try array.map { try resolve($0, visited, depth + 1) } }
                if let dictionary = value as? [String: Any] {
                    var result: [String: Any] = [:]
                    for (key, item) in dictionary where key != "$class" { result[key] = try resolve(item, visited, depth + 1) }
                    if let keys = result["NS.keys"] as? [String], let values = result["NS.objects"] as? [Any], keys.count == values.count {
                        var mapped: [String: Any] = [:]
                        for (key, item) in zip(keys, values) { mapped[key] = item }
                        return mapped
                    }
                    if let array = result["NS.objects"] as? [Any] { return array }
                    if let relative = result["NS.relative"] as? String { return relative }
                    if let string = result["NS.string"] as? String { return string }
                    return result
                }
                return value
            }
            return try resolve(top["root"] ?? top["NSKeyedArchiveRootObjectKey"] ?? NSNull(), [], 0) as? [String: Any]
        } catch { return nil }
    }
}
