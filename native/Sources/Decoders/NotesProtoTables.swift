import Foundation

extension NotesProto {
    static func table(_ data: Data, tables: [String: Data], depth: Int) throws -> [[NotesCell]] {
        guard depth < 8,
              let envelope = try NotesProtoMessage(NotesProtoWire.unpack(data)).message(2),
              let graph = try envelope.message(3) else { throw NotesProtoFailure.malformed }
        let objects = try graph.repeated(3).map(NotesProtoMessage.init)
        let keys = try graph.repeated(4).map { bytes -> String in
            guard let value = String(data: bytes, encoding: .utf8) else { throw NotesProtoFailure.malformed }
            return value
        }
        let types = try graph.repeated(5).map { bytes -> String in
            guard let value = String(data: bytes, encoding: .utf8) else { throw NotesProtoFailure.malformed }
            return value
        }
        let uuids = graph.repeated(6)
        func object(_ index: UInt64?) throws -> NotesProtoMessage {
            guard let index, index < objects.count else { throw NotesProtoFailure.malformed }
            return objects[Int(index)]
        }
        func target(_ value: NotesProtoMessage) throws -> UInt64 {
            guard let map = try value.message(13), let entry = try map.repeated(3).first.map(NotesProtoMessage.init),
                  let identifier = try entry.message(2)?.integer(2), identifier < uuids.count else { throw NotesProtoFailure.malformed }
            return identifier
        }
        func dictionary(_ value: NotesProtoMessage) throws -> [(NotesProtoMessage, NotesProtoMessage)] {
            guard let dictionary = try value.message(6) else { throw NotesProtoFailure.malformed }
            return try dictionary.repeated(1).map { bytes in
                let entry = try NotesProtoMessage(bytes)
                guard let key = try entry.message(1), let value = try entry.message(2) else { throw NotesProtoFailure.malformed }
                return (key, value)
            }
        }
        func ordering(_ value: NotesProtoMessage) throws -> (Int, [UInt64: Int]) {
            guard let set = try value.message(16), let ordering = try set.message(1), let array = try ordering.message(1) else { throw NotesProtoFailure.malformed }
            var entries: [(UInt64, Data)] = []
            for bytes in array.repeated(2) {
                let item = try NotesProtoMessage(bytes)
                guard let position = item.integer(1), let uuid = item.bytes(2), uuid.count == 16 else { throw NotesProtoFailure.malformed }
                entries.append((position, uuid))
            }
            entries.sort { $0.0 < $1.0 }
            guard entries.count <= 16384, Set(entries.map(\.0)).count == entries.count else { throw NotesProtoFailure.unsupported }
            var indices: [UInt64: Int] = [:]
            for (index, entry) in entries.enumerated() {
                guard let uuid = uuids.firstIndex(of: entry.1), indices[UInt64(uuid)] == nil else { throw NotesProtoFailure.malformed }
                indices[UInt64(uuid)] = index
            }
            if let aliases = try ordering.message(2) {
                var unresolved = try aliases.repeated(1).map { bytes -> (UInt64, UInt64) in
                    let pair = try NotesProtoMessage(bytes)
                    let key = try object(pair.message(1)?.integer(6))
                    let value = try object(pair.message(2)?.integer(6))
                    return try (target(key), target(value))
                }
                while !unresolved.isEmpty {
                    let previous = unresolved.count
                    unresolved = try unresolved.filter { key, value in
                        guard let position = indices[key] else { return true }
                        if let existing = indices[value], existing != position { throw NotesProtoFailure.malformed }
                        indices[value] = position
                        return false
                    }
                    guard unresolved.count < previous else { throw NotesProtoFailure.malformed }
                }
            }
            return (entries.count, indices)
        }
        var root: NotesProtoMessage?
        for value in objects {
            guard let map = try value.message(13), let type = map.integer(1), type < types.count else { continue }
            if types[Int(type)] == "com.apple.notes.ICTable" {
                guard root == nil else { throw NotesProtoFailure.unsupported }
                root = map
            }
        }
        guard let root else { throw NotesProtoFailure.unsupported }
        var parts: [String: NotesProtoMessage] = [:]
        for bytes in root.repeated(3) {
            let entry = try NotesProtoMessage(bytes)
            guard let key = entry.integer(1), key < keys.count, let value = try entry.message(2) else { throw NotesProtoFailure.malformed }
            guard ["crRows", "crColumns", "cellColumns"].contains(keys[Int(key)]) else { continue }
            parts[keys[Int(key)]] = try object(value.integer(6))
        }
        guard let rowObject = parts["crRows"], let columnObject = parts["crColumns"], let cells = parts["cellColumns"] else { throw NotesProtoFailure.malformed }
        let (rowCount, rows) = try ordering(rowObject)
        let (columnCount, columns) = try ordering(columnObject)
        guard rowCount > 0, columnCount > 0, rowCount <= 16384 / columnCount else { throw NotesProtoFailure.unsupported }
        var result = Array(repeating: Array(repeating: NotesCell(blocks: []), count: columnCount), count: rowCount)
        var occupied = Set<String>()
        for (key, value) in try dictionary(cells) {
            let column = try target(object(key.integer(6)))
            guard let columnIndex = columns[column] else { throw NotesProtoFailure.malformed }
            for (rowKey, rowValue) in try dictionary(object(value.integer(6))) {
                let row = try target(object(rowKey.integer(6)))
                guard let rowIndex = rows[row], occupied.insert("\(rowIndex):\(columnIndex)").inserted,
                      let note = try object(rowValue.integer(6)).message(10) else { throw NotesProtoFailure.malformed }
                result[rowIndex][columnIndex] = NotesCell(blocks: try decodeNote(note, tables: tables, depth: depth + 1).blocks)
            }
        }
        for value in objects {
            guard let map = try value.message(13) else { continue }
            for bytes in map.repeated(3) {
                let entry = try NotesProtoMessage(bytes)
                if let direction = try entry.message(2)?.string(4), direction == "CRTableColumnDirectionRightToLeft" {
                    result = result.map { Array($0.reversed()) }
                    return result
                }
            }
        }
        return result
    }
}
