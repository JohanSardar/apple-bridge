import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StoreEnrichmentChecks {
    static func run(path: String, execute: (String) -> Void) async throws -> [JSON] {
        let parent = "11111111-2222-3333-4444-555555555555"
        execute("UPDATE ZREMCDREMINDER SET ZDACALENDARITEMUNIQUEIDENTIFIER='\(parent)',ZFLAGGED=1,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        execute("UPDATE ZREMCDREMINDER SET ZDACALENDARITEMUNIQUEIDENTIFIER='child',ZMARKEDFORDELETION=0,Z_OPT=Z_OPT+1 WHERE Z_PK=11")
        func observation(_ collection: String, _ key: String, _ data: JSON) -> JSON {
            .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "data": data])
        }
        let parentData = JSON.object(["title": .string("Bridge Test Parent"), "completed": .bool(false), "list_id": RemindersStoreSnapshot.reference("reminders.list", "list")])
        let childData = JSON.object(["title": .string("Bridge Test Child"), "completed": .bool(false), "list_id": RemindersStoreSnapshot.reference("reminders.list", "list")])
        let native = [observation("reminders.list", "list", .object(["title": .string("Bridge Test List"), "writable": .bool(true)])), observation("reminders.reminder", parent, parentData), observation("reminders.reminder", "child", childData)]
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        let enriched = await enrichment.refresh(native, generation: "fixture")
        let parentObservation = enriched.first { $0["native_key"].string == parent }!
        let childObservation = enriched.first { $0["native_key"].string == "child" }!
        precondition(parentObservation["data"]["flagged"] == .bool(true))
        precondition(childObservation["data"]["parent_id"]["$ref"]["native_key"] == .string(parent))
        precondition(parentObservation["data"]["early_reminders"] == .null)
        precondition(RemindersStoreProjection.baseObservation(parentObservation)["data"] == parentData)
        execute("UPDATE ZREMCDREMINDER SET ZFLAGGED=0,Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let changed = await enrichment.refresh([], generation: "fixture")
        precondition(changed.count == 1 && changed[0]["data"]["flagged"] == .bool(false))
        let revalidated = await enrichment.apply([native[1]])
        precondition(revalidated[0]["data"]["flagged"] == .bool(false))
        execute("UPDATE ZREMCDREMINDER SET ZDACALENDARITEMUNIQUEIDENTIFIER='external-parent',ZEXTERNALIDENTIFIER='\(parent)',Z_OPT=Z_OPT+1 WHERE Z_PK=10")
        let unmatched = await enrichment.refresh([], generation: "fixture")
        precondition(unmatched.contains { $0["native_key"].string == parent && $0["data"]["flagged"] == .null && $0["data"]["store_status"] == .string("unavailable") })
        let unchanged = await enrichment.refresh([], generation: "fixture")
        precondition(unchanged.isEmpty)
        let newGeneration = await enrichment.refresh([], generation: "new-generation")
        precondition(newGeneration.isEmpty)
        print("capture_enrichment_store_only_changes_native_baseline_revalidation_exact_identity: PASS")
        return enriched
    }
}
