import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StoreSectionChecks {
    static func run(path: String, execute: (String) -> Void) async throws {
        let first = UUID(), second = UUID(), item = UUID()
        let order = try JSONEncoder().encode([first.uuidString, second.uuidString])
        let membership = try JSONEncoder().encode([item.uuidString: first.uuidString])
        precondition(StoreSections.decode(order: order, membership: membership)?.order == [first, second])
        precondition(StoreSections.decode(order: order, membership: nil)?.membership == [:])
        precondition(StoreSections.decode(order: order, membership: membership)?.membership == [item: first])
        let plistOrder = try PropertyListSerialization.data(fromPropertyList: [first.uuidString, second.uuidString], format: .binary, options: 0)
        let archive = try NSKeyedArchiver.archivedData(withRootObject: [item.uuidString: second.uuidString], requiringSecureCoding: true)
        precondition(StoreSections.decode(order: plistOrder, membership: archive)?.membership == [item: second])
        precondition(StoreSections.decode(order: Data("[]".utf8), membership: Data("{}".utf8))?.order == [])
        for malformed in ["null", "{}", "[1]", "[\"bad\"]", "[\"\(first)\",\"\(first)\"]"] {
            precondition(StoreSections.decode(order: Data(malformed.utf8), membership: membership) == nil)
        }
        let foreign = try JSONEncoder().encode([item.uuidString: UUID().uuidString])
        precondition(StoreSections.decode(order: order, membership: foreign) == nil)
        execute("ALTER TABLE ZREMCDBASELIST ADD COLUMN ZSECTIONIDSORDERINGASDATA BLOB")
        execute("DELETE FROM ZREMCDBASESECTION WHERE ZLIST=1")
        execute("INSERT INTO ZREMCDBASESECTION(Z_PK,Z_ENT,ZIDENTIFIER,ZDISPLAYNAME,ZLIST) VALUES(200,605,'\(first)','first',1),(201,605,'\(second)','second',1)")
        execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(202,101,'\(item)','\(item)',0,1)")
        func set(_ order: Data, _ members: Data) {
            let orderHex = order.map { String(format: "%02x", $0) }.joined()
            let membersHex = members.map { String(format: "%02x", $0) }.joined()
            execute("UPDATE ZREMCDBASELIST SET ZSECTIONIDSORDERINGASDATA=X'\(orderHex)',ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA=X'\(membersHex)',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        }
        set(order,membership)
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        let observations: [JSON] = [
            .object(["kind": .string("upsert"), "collection": .string("reminders.list"), "native_key": .string("list"), "data": .object([:])]),
            .object(["kind": .string("upsert"), "collection": .string("reminders.reminder"), "native_key": .string(item.uuidString), "data": .object([:])])
        ]
        let initial = await enrichment.refresh(observations, generation: "sections")
        precondition(initial.first { $0["collection"].string == "reminders.list" }?["data"]["sections"].array.map { $0["id"].string } == [first.uuidString,second.uuidString])
        precondition(initial.first { $0["collection"].string == "reminders.reminder" }?["data"]["section_id"].string == first.uuidString)
        set(try JSONEncoder().encode([second.uuidString,first.uuidString]),try JSONEncoder().encode([item.uuidString:second.uuidString]))
        let moved = await enrichment.refresh([],generation: "sections")
        precondition(moved.first { $0["collection"].string == "reminders.reminder" }?["data"]["section_id"].string == second.uuidString)
        execute("UPDATE ZREMCDBASESECTION SET ZDISPLAYNAME='renamed',Z_OPT=Z_OPT+1 WHERE Z_PK=201")
        let renamed = await enrichment.refresh([],generation: "sections")
        precondition(renamed.first { $0["collection"].string == "reminders.list" }?["data"]["sections"].array.first?["title"].string == "renamed")
        set(Data([1,2,3]),membership)
        let unsupported = await enrichment.refresh([],generation: "sections")
        precondition(unsupported.first { $0["collection"].string == "reminders.reminder" }?["data"]["section_id"] == .null)
        print("section checks passed")
    }
}
