import Foundation

public struct CompleteScan: Sendable {
    public let rows: [[String: SQLiteValue]]
    public let complete: Bool
    public let absentKeys: Set<String>

    public static func read(reader: SQLiteSnapshot, sql: String, values: [String] = [], keyColumn: String, scope: String, previousScope: String, previousGeneration: String, previousKeys: Set<String>, maxRows: Int, timeout: TimeInterval) throws -> CompleteScan {
        let result = try reader.query(sql, values: values, maxRows: maxRows, timeout: timeout)
        var keys = Set<String>()
        for row in result.rows {
            guard let key = row[keyColumn]?.text, !key.isEmpty, keys.insert(key).inserted else { throw SourceFailure.corrupt }
        }
        let complete = try result.complete && reader.isCurrent()
        let comparable = complete && !scope.isEmpty && scope == previousScope && reader.generation == previousGeneration
        return CompleteScan(rows: result.rows, complete: complete, absentKeys: absence(previous: previousKeys, current: keys, complete: comparable))
    }
}
