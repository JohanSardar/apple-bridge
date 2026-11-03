import Foundation
import Decoders

enum ProbeFailure: Error { case unavailable }
enum MessagesPins {
    static func read(at: URL?) throws -> [String] {
        guard let at else { throw ProbeFailure.unavailable }
        let data = try Data(contentsOf: at)
        let root = try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
        return (root?["pD"] as? [String: Any])?["pP"] as? [String] ?? []
    }
    static func index(_ value: String, in pins: [String]) -> Int? { value.isEmpty ? nil : pins.firstIndex(of: value) }
}
enum BlobDigest {
    static func isDataless(_ url: URL) -> Bool { false }
}
@main struct MessagesOracle {
    static func main() throws {
        let path = CommandLine.arguments[1]
        let home = URL(fileURLWithPath: CommandLine.arguments[2])
        var reader = try MessagesReader(path: path, pinsURL: home.appendingPathComponent("Library/Preferences/com.apple.messages.pinning.plist"), attachmentHome: home)
        var prior: [String: Data] = [:]
        var candidates: Set<String> = []
        while let request = readLine() {
            if try !reader.isCurrent() {
                let fresh = try MessagesReader(path: path, pinsURL: home.appendingPathComponent("Library/Preferences/com.apple.messages.pinning.plist"), attachmentHome: home)
                if fresh.generation != reader.generation {
                    candidates.formUnion(prior.keys)
                    prior.removeAll()
                }
                reader = fresh
            }
            var items = try reader.smallObservations()
            var position: Int64 = 0
            while true {
                let rows = try reader.messageRows(after: position, through: Int64.max)
                guard let last = rows.last else { break }
                position = last.integer("row_position")
                for row in rows { items += try reader.messageObservations(row) }
            }
            var seen: Set<String> = []
            var output: [[String: Any]] = []
            for (collection, key, data) in items where !key.isEmpty {
                let identity = collection + "/" + key
                seen.insert(identity)
                let bytes = try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys])
                if prior[identity] != bytes {
                    output.append(["kind": "upsert", "collection": collection, "native_key": key, "data": data])
                    prior[identity] = bytes
                }
            }
            if request == "complete" {
                for identity in Set(prior.keys).union(candidates).subtracting(seen).sorted() {
                    let fields = identity.split(separator: "/", maxSplits: 1).map(String.init)
                    let table = fields[0] == "messages.chat" ? "chat" : fields[0] == "messages.handle" ? "handle" : fields[0] == "messages.attachment" ? "attachment" : "message"
                    let required = table == "handle" ? ["id", "service"] : ["guid"]
                    guard required.allSatisfy({ reader.columns[table]?.contains($0) == true }) else { continue }
                    output.append(["kind": "absent", "collection": fields[0], "native_key": fields[1], "data": [:]])
                    prior.removeValue(forKey: identity)
                    candidates.remove(identity)
                }
            }
            candidates.subtract(seen)
            output.sort { a,b in
                let left = ["collection","native_key","kind"].map { a[$0] as? String ?? "" }
                let right = ["collection","native_key","kind"].map { b[$0] as? String ?? "" }
                return left.lexicographicallyPrecedes(right)
            }
            for item in output {
                let data = try JSONSerialization.data(withJSONObject: item, options: [.sortedKeys, .withoutEscapingSlashes])
                print(String(decoding: data, as: UTF8.self))
            }
            print("END")
            fflush(stdout)
        }
    }
}
