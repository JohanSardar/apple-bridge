import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StoreSectionChecks {
    static func run(path: String, execute: (String) -> Void) async throws {
        func check(_ name: String, _ passed: Bool) {
            if !passed {
                FileHandle.standardError.write(Data((name + " failed\n").utf8))
                exit(1)
            }
        }
        let first = UUID(), second = UUID(), item = UUID()
        let order = try JSONEncoder().encode([first.uuidString, second.uuidString])
        let membership = try JSONEncoder().encode([item.uuidString: first.uuidString])
        precondition(StoreSections.decode(order: order, membership: membership)?.order == [first, second])
        precondition(StoreSections.decode(order: order, membership: nil)?.membership == [:])
        precondition(StoreSections.decode(order: order, membership: membership)?.membership == [item: first])
        let nativeMembership = try JSONSerialization.data(withJSONObject: ["memberships": [["groupID": second.uuidString, "memberID": item.uuidString, "modifiedOn": 815202026.752328]], "minimumSupportedVersion": 20230430])
        precondition(StoreSections.decode(order: order, membership: nativeMembership)?.membership == [item: second])
        for entries in [
            [["groupID": UUID().uuidString, "memberID": item.uuidString]],
            [["groupID": first.uuidString, "memberID": item.uuidString], ["groupID": second.uuidString, "memberID": item.uuidString]],
            [["groupID": second.uuidString, "memberID": "invalid"]]
        ] {
            let invalid = try JSONSerialization.data(withJSONObject: ["memberships": entries, "minimumSupportedVersion": 20230430])
            precondition(StoreSections.decode(order: order, membership: invalid) == nil)
        }
        let futureMembership = try JSONSerialization.data(withJSONObject: ["memberships": [], "minimumSupportedVersion": 99999999])
        precondition(StoreSections.decode(order: order, membership: futureMembership) == nil)
        let nativeOrder = try JSONSerialization.data(withJSONObject: ["orderedIdentifiers": [second.uuidString, first.uuidString], "minimumSupportedVersion": 20230430])
        check("native section order envelope", StoreSections.decode(order: nativeOrder, membership: nativeMembership)?.order == [second, first])
        check("native section order membership", StoreSections.decode(order: nativeOrder, membership: nativeMembership)?.membership == [item: second])
        for invalidOrder: [String: Any] in [
            ["orderedIdentifiers": [first.uuidString], "minimumSupportedVersion": 99999999],
            ["orderedIdentifiers": [first.uuidString, first.uuidString], "minimumSupportedVersion": 20230430],
            ["orderedIdentifiers": "invalid", "minimumSupportedVersion": 20230430]
        ] {
            let invalid = try JSONSerialization.data(withJSONObject: invalidOrder)
            check("invalid section order envelope", StoreSections.decode(order: invalid, membership: nil) == nil)
        }
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
        execute("ALTER TABLE ZREMCDBASESECTION ADD COLUMN ZCREATIONDATE REAL")
        execute("DELETE FROM ZREMCDBASESECTION WHERE ZLIST=1")
        execute("INSERT INTO ZREMCDBASESECTION(Z_PK,Z_ENT,ZIDENTIFIER,ZDISPLAYNAME,ZLIST) VALUES(200,605,'\(first)','first',1),(201,605,'\(second)','second',1)")
        execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(202,101,'\(item)','\(item)',0,1)")
        func set(_ order: Data, _ members: Data) {
            let orderHex = order.map { String(format: "%02x", $0) }.joined()
            let membersHex = members.map { String(format: "%02x", $0) }.joined()
            execute("UPDATE ZREMCDBASELIST SET ZSECTIONIDSORDERINGASDATA=X'\(orderHex)',ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA=X'\(membersHex)',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        }
        set(order,membership)
        execute("UPDATE ZREMCDBASESECTION SET ZCREATIONDATE=CASE Z_PK WHEN 200 THEN 100 ELSE 200 END WHERE ZLIST=1")
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        let observations: [JSON] = [
            .object(["kind": .string("upsert"), "collection": .string("reminders.list"), "native_key": .string("list"), "data": .object([:])]),
            .object(["kind": .string("upsert"), "collection": .string("reminders.reminder"), "native_key": .string(item.uuidString), "data": .object([:])])
        ]
        let initial = await enrichment.refresh(observations, generation: "sections")
        precondition(initial.first { $0["collection"].string == "reminders.list" }?["data"]["sections"].array.map { $0["id"].string } == [first.uuidString,second.uuidString])
        precondition(initial.first { $0["collection"].string == "reminders.reminder" }?["data"]["section_id"].string == first.uuidString)
        execute("UPDATE ZREMCDBASELIST SET ZSECTIONIDSORDERINGASDATA=NULL,Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        let implicit = await enrichment.refresh(observations, generation: "sections")
        check("implicit section order", implicit.first { $0["collection"].string == "reminders.list" }?["data"]["sections"].array.map { $0["id"].string } == [first.uuidString, second.uuidString])
        check("implicit section membership", implicit.first { $0["collection"].string == "reminders.reminder" }?["data"]["section_id"].string == first.uuidString)
        execute("UPDATE ZREMCDBASESECTION SET ZCREATIONDATE=100,Z_OPT=Z_OPT+1 WHERE Z_PK=201")
        let ambiguous = await enrichment.refresh([], generation: "sections")
        check("ambiguous section order", ambiguous.first { $0["collection"].string == "reminders.list" }?["data"]["sections"] == .null)
        execute("UPDATE ZREMCDBASESECTION SET ZCREATIONDATE=200,Z_OPT=Z_OPT+1 WHERE Z_PK=201")
        set(nativeOrder,try JSONEncoder().encode([item.uuidString:second.uuidString]))
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
