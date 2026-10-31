import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StoreParityChecks {
    static func run(path: String, execute: (String) -> Void) async throws {
        execute("ALTER TABLE ZREMCDOBJECT ADD COLUMN ZREMINDER4 INTEGER")
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        let observations: [JSON] = [
            .object(["kind": .string("upsert"), "collection": .string("reminders.reminder"), "native_key": .string("eventkit-child"), "data": .object([:])])
        ]
        let session = try RemindersStoreSession(path: path)
        let initial = try await session.read().snapshot
        precondition(StoreTags.names(for: 11, snapshot: initial) == ["bridge-test"])
        precondition(StoreTags.names(for: 10, snapshot: initial) == [])
        execute("UPDATE ZREMCDREMINDER SET ZDACALENDARITEMUNIQUEIDENTIFIER='child',Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        let childObservation = observations.map { value -> JSON in var result = value.object; result["native_key"] = .string("child"); return .object(result) }
        let captured = await enrichment.refresh(childObservation, generation: "parity")
        precondition(captured.first?["data"]["tags"] == .array([.string("bridge-test")]))
        execute("UPDATE ZREMCDHASHTAGLABEL SET ZNAME='ab-test-changed' WHERE Z_PK=40")
        let renamed = await enrichment.refresh([], generation: "parity")
        precondition(renamed.first?["data"]["tags"] == .array([.string("ab-test-changed")]))
        execute("UPDATE ZREMCDOBJECT SET ZREMINDER3=10,Z_OPT=Z_OPT+1 WHERE Z_PK=30")
        let moved = await enrichment.refresh([], generation: "parity")
        precondition(moved.first?["data"]["tags"] == .array([]))
        execute("UPDATE ZREMCDOBJECT SET ZREMINDER3=11,ZREMINDER4=10,Z_OPT=Z_OPT+1 WHERE Z_PK=30")
        let ambiguous = try await session.read().snapshot
        precondition(StoreTags.names(for: 11, snapshot: ambiguous) == nil)
        print("parity checks passed")
    }
}
