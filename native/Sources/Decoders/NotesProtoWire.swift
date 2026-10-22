import Foundation
import zlib

struct NotesProtoField {
    let number: Int
    let wire: UInt8
    let value: UInt64
    let bytes: Data
}

struct NotesProtoMessage {
    let fields: [NotesProtoField]
    init(_ data: Data) throws { fields = try NotesProtoWire.fields(data) }
    func validate(_ types: [Int: UInt8], repeated: Set<Int> = []) throws {
        var seen = Set<Int>()
        for field in fields {
            guard let wire = types[field.number] else { continue }
            guard wire == field.wire, repeated.contains(field.number) || seen.insert(field.number).inserted else { throw NotesProtoFailure.malformed }
        }
    }
    func bytes(_ number: Int) -> Data? { fields.first { $0.number == number && $0.wire == 2 }?.bytes }
    func integer(_ number: Int) -> UInt64? { fields.first { $0.number == number && $0.wire == 0 }?.value }
    func repeated(_ number: Int) -> [Data] { fields.filter { $0.number == number && $0.wire == 2 }.map(\.bytes) }
    func string(_ number: Int) throws -> String? {
        guard let data = bytes(number) else { return nil }
        guard let value = String(data: data, encoding: .utf8) else { throw NotesProtoFailure.malformed }
        return value
    }
    func message(_ number: Int) throws -> NotesProtoMessage? {
        try bytes(number).map(NotesProtoMessage.init)
    }
}

enum NotesProtoFailure: Error { case malformed, unsupported }

enum NotesProtoWire {
    static func fields(_ input: Data) throws -> [NotesProtoField] {
        guard input.count <= 16 * 1024 * 1024 else { throw NotesProtoFailure.malformed }
        let data = [UInt8](input)
        var offset = 0
        func varint() throws -> UInt64 {
            var result: UInt64 = 0
            for shift in stride(from: 0, through: 63, by: 7) {
                guard offset < data.count else { throw NotesProtoFailure.malformed }
                let byte = data[offset]
                offset += 1
                guard shift < 63 || byte <= 1 else { throw NotesProtoFailure.malformed }
                result |= UInt64(byte & 127) << shift
                if byte < 128 { return result }
            }
            throw NotesProtoFailure.malformed
        }
        var result: [NotesProtoField] = []
        while offset < data.count {
            let key = try varint(), wire = UInt8(key & 7)
            guard key >> 3 > 0, key >> 3 <= 0x1fffffff else { throw NotesProtoFailure.malformed }
            var value: UInt64 = 0
            var bytes = Data()
            switch wire {
            case 0: value = try varint()
            case 1, 5:
                let count = wire == 1 ? 8 : 4
                guard count <= data.count - offset else { throw NotesProtoFailure.malformed }
                bytes = Data(data[offset..<offset + count])
                offset += count
            case 2:
                let length = try varint()
                guard length <= UInt64(data.count - offset) else { throw NotesProtoFailure.malformed }
                bytes = Data(data[offset..<offset + Int(length)])
                offset += Int(length)
            default: throw NotesProtoFailure.malformed
            }
            result.append(NotesProtoField(number: Int(key >> 3), wire: wire, value: value, bytes: bytes))
        }
        return result
    }

    static func unpack(_ data: Data) throws -> Data {
        guard data.count <= 16 * 1024 * 1024 else { throw NotesProtoFailure.malformed }
        guard data.starts(with: [0x1f, 0x8b]) else { return data }
        var stream = z_stream()
        guard inflateInit2_(&stream, 47, ZLIB_VERSION, Int32(MemoryLayout<z_stream>.size)) == Z_OK else { throw NotesProtoFailure.malformed }
        defer { inflateEnd(&stream) }
        return try data.withUnsafeBytes { input in
            stream.next_in = UnsafeMutablePointer(mutating: input.bindMemory(to: Bytef.self).baseAddress)
            stream.avail_in = uInt(input.count)
            var output = Data()
            var buffer = [UInt8](repeating: 0, count: 65536)
            while true {
                let status = buffer.withUnsafeMutableBytes { bytes in
                    stream.next_out = bytes.bindMemory(to: Bytef.self).baseAddress
                    stream.avail_out = uInt(bytes.count)
                    return zlib.inflate(&stream, Z_NO_FLUSH)
                }
                output.append(contentsOf: buffer.prefix(buffer.count - Int(stream.avail_out)))
                guard output.count <= 16 * 1024 * 1024 else { throw NotesProtoFailure.malformed }
                if status == Z_STREAM_END { return output }
                guard status == Z_OK else { throw NotesProtoFailure.malformed }
            }
        }
    }
}
