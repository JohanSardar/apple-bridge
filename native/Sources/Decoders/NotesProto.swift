import Foundation
import zlib

public struct NotesBody: Sendable {
    public let text: String
    public let rich: Bool
    public let attachments: [String]
}

public enum NotesProto {
    private struct Field {
        let number: Int
        let value: UInt64
        let bytes: Data
    }
    private static func fields(_ input: Data) throws -> [Field] {
        let data = [UInt8](input)
        var offset = 0
        func varint() throws -> UInt64 {
            var result: UInt64 = 0
            for shift in stride(from: 0, through: 63, by: 7) {
                guard offset < data.count else { throw DecodeFailure.malformed }
                let byte = data[offset]; offset += 1
                guard shift < 63 || byte <= 1 else { throw DecodeFailure.malformed }
                result |= UInt64(byte & 127) << shift
                if byte < 128 { return result }
            }
            throw DecodeFailure.malformed
        }
        var result: [Field] = []
        while offset < data.count {
            let key = try varint()
            guard key >> 3 > 0 else { throw DecodeFailure.malformed }
            var value: UInt64 = 0
            var bytes = Data()
            switch key & 7 {
            case 0: value = try varint()
            case 1, 5:
                let count = key & 7 == 1 ? 8 : 4
                guard count <= data.count - offset else { throw DecodeFailure.malformed }
                bytes = Data(data[offset..<offset + count]); offset += count
            case 2:
                let length = try varint()
                guard length <= UInt64(data.count - offset) else { throw DecodeFailure.malformed }
                bytes = Data(data[offset..<offset + Int(length)]); offset += Int(length)
            default: throw DecodeFailure.malformed
            }
            result.append(Field(number: Int(key >> 3), value: value, bytes: bytes))
        }
        return result
    }
    private static func inflate(_ data: Data) throws -> Data {
        var stream = z_stream()
        guard inflateInit2_(&stream, 47, ZLIB_VERSION, Int32(MemoryLayout<z_stream>.size)) == Z_OK else { throw DecodeFailure.malformed }
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
                guard output.count <= 16 * 1024 * 1024 else { throw DecodeFailure.malformed }
                if status == Z_STREAM_END { return output }
                guard status == Z_OK else { throw DecodeFailure.malformed }
            }
        }
    }
    public static func decode(_ data: Data) throws -> NotesBody {
        let raw = data.starts(with: [0x1f, 0x8b]) ? try inflate(data) : data
        let envelope = try fields(raw)
        guard let document = envelope.first(where: { $0.number == 2 && !$0.bytes.isEmpty }), let note = try fields(document.bytes).first(where: { $0.number == 3 && !$0.bytes.isEmpty }) else { throw DecodeFailure.malformed }
        let body = try fields(note.bytes)
        guard let textField = body.first(where: { $0.number == 2 }), let text = String(data: textField.bytes, encoding: .utf8) else { throw DecodeFailure.malformed }
        var rich = text.contains("\u{fffc}")
        var attachments: [String] = []
        for run in body where run.number == 5 {
            for attribute in try fields(run.bytes) {
                if attribute.number == 12 {
                    rich = true
                    for item in try fields(attribute.bytes) where item.number == 1 {
                        if let identifier = String(data: item.bytes, encoding: .utf8) { attachments.append(identifier) }
                    }
                }
                if attribute.number == 2 {
                    let paragraph = try fields(attribute.bytes)
                    if paragraph.contains(where: { ($0.number == 1 && $0.value >= 100) || $0.number == 5 }) { rich = true }
                }
            }
        }
        return NotesBody(text: text, rich: rich, attachments: attachments)
    }
    private enum DecodeFailure: Error { case malformed }
}
