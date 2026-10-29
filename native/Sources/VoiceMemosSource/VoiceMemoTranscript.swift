import Foundation
import SourceSupport

public enum VoiceMemoTranscript {
    public static func read(_ file: URL) throws -> [JSON] {
        let input = try FileHandle(forReadingFrom: file)
        defer { try? input.close() }
        let size = try input.seekToEnd()
        func integer(_ data: Data) -> UInt64 { data.reduce(0) { ($0 << 8) | UInt64($1) } }
        func atoms(_ start: UInt64, _ end: UInt64, _ depth: Int) throws -> [JSON] {
            guard depth < 8 else { return [] }
            var offset = start
            while offset <= end, end - offset >= 8 {
                try input.seek(toOffset: offset)
                guard let header = try input.read(upToCount: 8), header.count == 8 else { return [] }
                var length = integer(header.prefix(4))
                let kind = String(data: header.suffix(4), encoding: .ascii) ?? ""
                var headerSize: UInt64 = 8
                if length == 1 {
                    guard let extended = try input.read(upToCount: 8), extended.count == 8 else { return [] }
                    length = integer(extended); headerSize = 16
                }
                if length == 0 { length = end - offset }
                guard length >= headerSize, length <= end - offset else { return [] }
                if kind == "tsrp", length - headerSize <= 4 * 1024 * 1024 {
                    guard let data = try input.read(upToCount: Int(length - headerSize)) else { return [] }
                    return try decode(data)
                }
                if ["moov", "trak", "udta"].contains(kind) {
                    let result = try atoms(offset + headerSize, offset + length, depth + 1)
                    if !result.isEmpty { return result }
                }
                offset += length
            }
            return []
        }
        return try atoms(0, size, 0)
    }
    public static func decode(_ data: Data) throws -> [JSON] {
        let value = try JSONDecoder().decode(JSON.self, from: data)
        var segments: [JSON] = []
        func append(_ text: String, _ range: JSON) {
            let times = range.array
            guard !text.isEmpty, times.count == 2,
                  times.allSatisfy({ if case .number(let n) = $0 { return n.isFinite }; if case .integer = $0 { return true }; return false }),
                  times[0].number >= 0, times[1].number >= times[0].number else { return }
            segments.append(.object(["text": .string(text), "start": times[0], "end": times[1]]))
        }
        func visit(_ node: JSON, _ depth: Int) {
            guard depth < 16, segments.count < 100000 else { return }
            if case .object(let fields) = node {
                let runs = fields["runs"]?.array ?? []
                let table = fields["attributeTable"]?.array ?? []
                if !table.isEmpty {
                    var index = 0
                    while index + 1 < runs.count {
                        if case .string(let text) = runs[index], case .integer(let attribute) = runs[index + 1], attribute < UInt64(table.count) {
                            append(text, table[Int(attribute)]["timeRange"])
                        }
                        index += 2
                    }
                }
                if let text = fields["text"] { append(text.string, fields["timeRange"] ?? fields["attributes"]?["timeRange"] ?? .null) }
                for key in fields.keys.sorted() where key != "attributes" { visit(fields[key]!, depth + 1) }
            }
            if case .array(let values) = node {
                var index = 0
                while index < values.count {
                    if index + 1 < values.count, case .string(let text) = values[index], case .object = values[index + 1] {
                        append(text, values[index + 1]["timeRange"])
                        index += 2
                    } else { visit(values[index], depth + 1); index += 1 }
                }
            }
        }
        visit(value, 0)
        return segments.sorted { $0["start"].number < $1["start"].number }
    }
}
