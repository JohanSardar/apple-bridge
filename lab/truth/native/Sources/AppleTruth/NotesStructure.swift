import Foundation
import zlib

enum NotesTruthStructure {
    private struct Field {
        var tag: Int
        var wire: UInt64
        var value: UInt64 = 0
        var bytes = Data()
    }
    private enum Invalid: Error { case protobuf }
    private static func fields(_ data: Data) throws -> [Field] {
        let bytes = [UInt8](data)
        var index = 0, result: [Field] = []
        func unsigned() throws -> UInt64 {
            var value: UInt64 = 0
            for shift in stride(from: 0, through: 63, by: 7) {
                guard index < bytes.count else { throw Invalid.protobuf }
                let byte = bytes[index]; index += 1
                guard shift < 63 || byte <= 1 else { throw Invalid.protobuf }
                value |= UInt64(byte & 127) << shift
                if byte < 128 { return value }
            }
            throw Invalid.protobuf
        }
        while index < bytes.count {
            let header = try unsigned()
            guard header >> 3 > 0, header >> 3 <= 0x1fffffff else { throw Invalid.protobuf }
            var field = Field(tag: Int(header >> 3), wire: header & 7)
            if field.wire == 0 { field.value = try unsigned() }
            else {
                let size: UInt64
                switch field.wire {
                case 1: size = 8
                case 2: size = try unsigned()
                case 5: size = 4
                default: throw Invalid.protobuf
                }
                guard size <= UInt64(bytes.count - index) else { throw Invalid.protobuf }
                field.bytes = Data(bytes[index..<index + Int(size)])
                index += Int(size)
            }
            result.append(field)
        }
        return result
    }
    static func unpack(_ input: Data) throws -> Data {
        var raw = input
        if input.starts(with: [31,139]) {
            var state = z_stream()
            guard inflateInit2_(&state, 31, ZLIB_VERSION, Int32(MemoryLayout<z_stream>.size)) == Z_OK else { throw Invalid.protobuf }
            defer { inflateEnd(&state) }
            raw = try input.withUnsafeBytes { bytes in
                state.next_in = UnsafeMutablePointer(mutating: bytes.bindMemory(to: Bytef.self).baseAddress); state.avail_in = uInt(bytes.count)
                var result = Data(), block = [UInt8](repeating: 0, count: 8192)
                while true {
                    let code = block.withUnsafeMutableBytes { destination in
                        state.next_out = destination.bindMemory(to: Bytef.self).baseAddress; state.avail_out = uInt(destination.count)
                        return inflate(&state, Z_NO_FLUSH)
                    }
                    result.append(contentsOf: block.prefix(block.count - Int(state.avail_out)))
                    guard result.count < 32 * 1024 * 1024 else { throw Invalid.protobuf }
                    if code == Z_STREAM_END { return result }
                    guard code == Z_OK else { throw Invalid.protobuf }
                }
            }
        }
        return raw
    }
    static func count(_ note: Data) throws -> [String: Int] {
        let entries = try fields(note)
        guard let text = entries.first(where: { $0.tag == 2 && $0.wire == 2 }), let decoded = String(data: text.bytes, encoding: .utf8) else { throw Invalid.protobuf }
        let units = Array(decoded.utf16)
        var offset = 0, paragraph = 0
        var styles: [Int: Int] = [:], checked: [Int: Bool] = [:]
        var quotes = Set<Int>(), highlights = Set<Int>(), links = Set<String>(), attachments = Set<String>(), tables = Set<String>()
        for entry in entries where entry.tag == 5 && entry.wire == 2 {
            let run = try fields(entry.bytes)
            guard let length = run.first(where: { $0.tag == 1 && $0.wire == 0 })?.value, length <= UInt64(units.count - offset) else { throw Invalid.protobuf }
            let format = try run.first(where: { $0.tag == 2 && $0.wire == 2 }).map { try fields($0.bytes) } ?? []
            let style = format.first(where: { $0.tag == 1 && $0.wire == 0 }).map { Int(Int32(truncatingIfNeeded: $0.value)) }
            let checklist = try format.first(where: { $0.tag == 5 && $0.wire == 2 }).map { try fields($0.bytes) } ?? []
            let done = checklist.first(where: { $0.tag == 2 && $0.wire == 0 })?.value == 1
            for position in offset..<offset + Int(length) {
                if let style { styles[paragraph] = style }
                if style == 103 { checked[paragraph] = done }
                if format.contains(where: { $0.tag == 8 && $0.wire == 0 && $0.value == 1 }) { quotes.insert(paragraph) }
                if run.contains(where: { $0.tag == 14 && $0.wire == 0 && $0.value > 0 }) { highlights.insert(paragraph) }
                if units[position] == 10 { paragraph += 1 }
            }
            offset += Int(length)
            if let link = run.first(where: { $0.tag == 9 && $0.wire == 2 }), let url = String(data: link.bytes, encoding: .utf8) { links.insert(url) }
            if let attachment = run.first(where: { $0.tag == 12 && $0.wire == 2 }) {
                let metadata = try fields(attachment.bytes)
                if let id = metadata.first(where: { $0.tag == 1 && $0.wire == 2 }), let identifier = String(data: id.bytes, encoding: .utf8) {
                    attachments.insert(identifier)
                    if metadata.contains(where: { $0.tag == 2 && String(data: $0.bytes, encoding: .utf8) == "com.apple.notes.table" }) { tables.insert(identifier) }
                }
            }
        }
        return ["titles": styles.values.filter { $0 == 0 }.count, "headings": styles.values.filter { $0 == 1 || $0 == 2 }.count,
                "monostyled": styles.values.filter { $0 == 4 }.count, "list_items": styles.values.filter { (100...102).contains($0) }.count,
                "checklists": checked.count, "checked": checked.values.filter { $0 }.count, "quotes": quotes.count,
                "highlighted_paragraphs": highlights.count, "links": links.count, "attachments": attachments.count, "tables": tables.count]
    }
    static func tableIDs(_ note: Data) throws -> [String] {
        var result = Set<String>()
        for run in try fields(note) where run.tag == 5 && run.wire == 2 {
            for attachment in try fields(run.bytes) where attachment.tag == 12 && attachment.wire == 2 {
                let metadata = try fields(attachment.bytes)
                guard metadata.contains(where: { $0.tag == 2 && String(data: $0.bytes, encoding: .utf8) == "com.apple.notes.table" }),
                      let id = metadata.first(where: { $0.tag == 1 }), let value = String(data: id.bytes, encoding: .utf8) else { continue }
                result.insert(value)
            }
        }
        return result.sorted()
    }
    static func tableCells(_ input: Data) throws -> Int {
        func child(_ data: Data, _ tag: Int) throws -> Data {
            guard let item = try fields(data).first(where: { $0.tag == tag && $0.wire == 2 }) else { throw Invalid.protobuf }
            return item.bytes
        }
        let graph = try fields(child(child(unpack(input), 2), 3))
        let objects = graph.filter { $0.tag == 3 && $0.wire == 2 }.map(\.bytes)
        let keys = graph.filter { $0.tag == 4 && $0.wire == 2 }.map { String(data: $0.bytes, encoding: .utf8) ?? "" }
        let types = graph.filter { $0.tag == 5 && $0.wire == 2 }.map { String(data: $0.bytes, encoding: .utf8) ?? "" }
        var dimensions: [String: Int] = [:]
        for object in objects {
            guard let map = try fields(object).first(where: { $0.tag == 13 && $0.wire == 2 }) else { continue }
            let entries = try fields(map.bytes)
            guard let type = entries.first(where: { $0.tag == 1 && $0.wire == 0 })?.value, type < types.count,
                  types[Int(type)] == "com.apple.notes.ICTable" else { continue }
            for entry in entries where entry.tag == 3 && entry.wire == 2 {
                let pair = try fields(entry.bytes)
                guard let key = pair.first(where: { $0.tag == 1 && $0.wire == 0 })?.value, key < keys.count else { throw Invalid.protobuf }
                let name = keys[Int(key)]
                guard name == "crRows" || name == "crColumns" else { continue }
                let target = try fields(child(entry.bytes, 2)).first { $0.tag == 6 && $0.wire == 0 }?.value
                guard let target, target < objects.count, dimensions[name] == nil else { throw Invalid.protobuf }
                let array = try fields(child(child(child(objects[Int(target)], 16), 1), 1))
                let attachments = array.filter { $0.tag == 2 && $0.wire == 2 }
                let ids = try attachments.map { try child($0.bytes, 2) }
                guard !ids.isEmpty, ids.allSatisfy({ $0.count == 16 }), Set(ids).count == ids.count else { throw Invalid.protobuf }
                dimensions[name] = ids.count
            }
        }
        guard let rows = dimensions["crRows"], let columns = dimensions["crColumns"], rows <= 16384 / columns else { throw Invalid.protobuf }
        return rows * columns
    }
}
