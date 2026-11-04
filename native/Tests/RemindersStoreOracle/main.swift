import Foundation
import SQLite3
import RemindersStore
import SourceSupport

@main
struct RemindersStoreOracle {
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count == 3 else { throw SourceFailure.invalid }
        let path = arguments[1]
        let steps = try JSONDecoder().decode([JSON].self, from: Data(contentsOf: URL(fileURLWithPath: arguments[2])))
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_close(database) }
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        var results: [JSON] = []
        for step in steps {
            let sql = step["sql"].string
            if !sql.isEmpty, sqlite3_exec(database, sql, nil, nil, nil) != SQLITE_OK { throw SourceFailure.invalid }
            let refreshed = await enrichment.refresh(step["observations"].array, generation: "oracle")
            let stripped = refreshed.map { observation -> JSON in
                var object = observation.object
                object.removeValue(forKey: "observed_at")
                return .object(object)
            }
            results.append(.object(["label": step["label"], "observations": .array(stripped)]))
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .prettyPrinted, .withoutEscapingSlashes]
        print(String(decoding: try encoder.encode(JSON.array(results)), as: UTF8.self))
    }
}
