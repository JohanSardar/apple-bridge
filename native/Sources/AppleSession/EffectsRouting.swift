import Foundation
import NativeProtocol
import SourceSupport

struct EffectsRouting: Sendable {
    static let file = "session/rust-effects.json"
    let sources: Set<String>?

    static func load(home: URL) -> EffectsRouting? {
        let environment = ProcessInfo.processInfo.environment
        guard environment["AB_SESSION_PROXY"] == "1" else { return nil }
        if let list = environment["AB_SESSION_EFFECTS_SOURCES"] { return parse(list) }
        if environment["AB_SESSION_EFFECTS"] == "1" { return EffectsRouting(sources: nil) }
        guard let bytes = try? Data(contentsOf: home.appendingPathComponent(file)), let value = try? JSONDecoder().decode(JSON.self, from: bytes), case .array(let names) = value["sources"] else { return nil }
        return EffectsRouting(sources: Set(names.map(\.string)))
    }

    static func parse(_ list: String) -> EffectsRouting {
        let names = Set(list.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })
        return EffectsRouting(sources: names.isEmpty || names.contains("*") ? nil : names)
    }

    static func source(of operation: String) -> String {
        if ["mac.screen_record", "print.document"].contains(operation) { return "capture" }
        return operation.components(separatedBy: ".").first ?? ""
    }

    func covers(_ operation: String) -> Bool {
        sources?.contains(Self.source(of: operation)) ?? true
    }

    func routes(_ operation: String) -> Bool {
        covers(operation) && NativeServer.effectOperations.contains(operation)
    }

    static let readOnlyQueries: Set<String> = ["files.search", "files.read"]

    func narrowed(_ capabilities: [String]) -> [String] {
        capabilities.filter { !covers($0) || NativeServer.effectOperations.contains($0) || Self.readOnlyQueries.contains($0) }
    }
}
