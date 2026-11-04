import Foundation
import CalendarSource
import SourceSupport

@main
struct CalendarStoreOracle {
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count == 4 else { throw SourceFailure.invalid }
        let root = URL(fileURLWithPath: arguments[1], isDirectory: true)
        let decoder = JSONDecoder()
        let observations = try decoder.decode([JSON].self, from: Data(contentsOf: URL(fileURLWithPath: arguments[2])))
        let lookups = try decoder.decode([String: JSON].self, from: Data(contentsOf: URL(fileURLWithPath: arguments[3])))
        var keyed: [String: JSON] = [:]
        for observation in observations { keyed[observation["collection"].string + "/" + observation["native_key"].string] = observation }
        let enrichment = CalendarStoreEnrichment(root: root)
        let enriched = await enrichment.enrich(keyed, lookups: lookups, force: true)
        let handles = await enrichment.handles
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .prettyPrinted, .withoutEscapingSlashes]
        let output: JSON = .object(["observations": .array(enriched.keys.sorted().compactMap { enriched[$0] }), "blob_digests": .array(handles.keys.sorted().map(JSON.string))])
        print(String(decoding: try encoder.encode(output), as: UTF8.self))
    }
}
