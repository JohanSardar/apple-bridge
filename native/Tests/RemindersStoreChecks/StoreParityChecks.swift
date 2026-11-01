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
        execute("UPDATE ZREMCDREMINDER SET ZFLAGGED=1,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        execute("UPDATE ZREMCDREMINDER SET ZPARENTREMINDER=10,Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        execute("UPDATE ZREMCDBASELIST SET ZBADGEEMBLEM='flag',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        let changed = try await session.read().snapshot
        precondition(changed.reminders[10]?.flagged == true && changed.reminders[11]?.parent == 10 && changed.lists[1]?.emblem == "flag")
        let parentID = "11111111-2222-3333-4444-555555555555"
        execute("UPDATE ZREMCDREMINDER SET ZDACALENDARITEMUNIQUEIDENTIFIER='\(parentID)',Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let parityObservations: [JSON] = [
            .object(["kind": .string("upsert"), "collection": .string("reminders.reminder"), "native_key": .string(parentID), "data": .object([:])]),
            .object(["kind": .string("upsert"), "collection": .string("reminders.list"), "native_key": .string("list"), "data": .object([:])])
        ]
        let flagged = await enrichment.refresh(parityObservations, generation: "parity")
        precondition(flagged.first { $0["collection"].string == "reminders.reminder" }?["data"]["flagged"] == .bool(true))
        execute("UPDATE ZREMCDREMINDER SET ZFLAGGED=0,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        execute("UPDATE ZREMCDBASELIST SET ZBADGEEMBLEM='star',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        let cleared = await enrichment.refresh([], generation: "parity")
        precondition(cleared.first { $0["native_key"].string == parentID }?["data"]["flagged"] == .bool(false))
        precondition(cleared.first { $0["collection"].string == "reminders.list" }?["data"]["badge_emblem"] == .string("star"))
        execute("UPDATE ZREMCDREMINDER SET ZPARENTREMINDER=NULL,Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        let outdented = try await session.read().snapshot
        precondition(outdented.reminders[11]?.parent == nil)
        execute("ALTER TABLE ZREMCDBASELIST ADD COLUMN ZREMINDERIDSMERGEABLEORDERING_V2_JSON BLOB")
        execute("ALTER TABLE ZREMCDBASELIST ADD COLUMN ZSMARTLISTTYPE INTEGER")
        execute("ALTER TABLE ZREMCDBASELIST ADD COLUMN ZFILTERDATA BLOB")
        let first = UUID(), second = UUID()
        execute("UPDATE ZREMCDREMINDER SET ZIDENTIFIER=X'\(first.uuidString.replacingOccurrences(of: "-", with: ""))',Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        execute("UPDATE ZREMCDREMINDER SET ZIDENTIFIER='\(second)',Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        let order = try JSONEncoder().encode([second.uuidString, first.uuidString]).map { String(format: "%02x", $0) }.joined()
        execute("UPDATE ZREMCDBASELIST SET ZREMINDERIDSMERGEABLEORDERING_V2_JSON=X'\(order)',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        execute("INSERT INTO ZREMCDOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZREMINDER3) VALUES(3003,934,'\(UUID())',11)")
        execute("INSERT INTO ZREMCDBASELIST(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME,ZSMARTLISTTYPE,ZFILTERDATA) VALUES(3004,408,'\(UUID())','[ab-test] smart',1,X'7b7d')")
        let parity = try await session.read().snapshot
        precondition(StoreManualOrder.decode(parity.lists[1]?.orderingJSON) == .supported([second: 0, first: 1]))
        precondition(parity.reminders[10]?.identity.uuid == first && parity.reminders[11]?.identity.textIdentifier == second.uuidString)
        precondition(StoreManualOrder.position(of: parity.reminders[10]!.identity, in: [second: 0, first: 1]) == 1)
        precondition(StoreManualOrder.position(of: parity.reminders[11]!.identity, in: [second: 0, first: 1]) == 0)
        precondition(parity.objects[3003]?.reminder3 == 11 && parity.lists[3004]?.smartType == .integer(1) && parity.lists[3004]?.filter == Data("{}".utf8))
        execute("DELETE FROM ZREMCDOBJECT WHERE Z_PK=3003")
        execute("DELETE FROM ZREMCDBASELIST WHERE Z_PK=3004")
        let cleaned = try await session.read().snapshot
        precondition(cleaned.objects[3003] == nil && cleaned.lists[3004] == nil)
        print("parity checks passed")
    }
}
