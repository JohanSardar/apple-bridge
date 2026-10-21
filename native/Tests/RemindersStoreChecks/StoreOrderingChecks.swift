import Foundation
import RemindersStore
import SourceSupport

@MainActor
enum StoreOrderingChecks {
    static func run(path: String, execute: (String) -> Void) async throws -> Bool {
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Fixtures/manual-order-uuid-array.json")
        let data = try Data(contentsOf: fixture)
        let identifiers = try JSONDecoder().decode([UUID].self, from: data)
        guard pureChecks(data: data, identifiers: identifiers) else { return false }
        guard identifiers.count == 3 else { return false }
        execute("ALTER TABLE ZREMCDBASELIST ADD COLUMN ZREMINDERIDSMERGEABLEORDERING_V2_JSON BLOB")
        let hexadecimal = data.map { String(format: "%02x", $0) }.joined()
        execute("UPDATE ZREMCDBASELIST SET ZREMINDERIDSMERGEABLEORDERING_V2_JSON=X'\(hexadecimal)',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        var observations: [JSON] = [.object(["kind": .string("upsert"), "collection": .string("reminders.list"), "native_key": .string("list"), "data": .object(["title": .string("Bridge Test Ordering")])])]
        for (index, identifier) in identifiers.enumerated() {
            let id = identifier.uuidString
            execute("INSERT INTO ZREMCDREMINDER(Z_PK,Z_ENT,ZIDENTIFIER,ZDACALENDARITEMUNIQUEIDENTIFIER,ZFLAGGED,ZLIST) VALUES(\(1000 + index),101,'\(id)','\(id)',0,1)")
            observations.append(.object(["kind": .string("upsert"), "collection": .string("reminders.reminder"), "native_key": .string(id), "data": .object(["title": .string("Bridge Test Ordering \(index)"), "list_id": RemindersStoreSnapshot.reference("reminders.list", "list")])]))
        }
        let session = try RemindersStoreSession(path: path)
        let native = try await session.read()
        let bytesExact = native.snapshot.lists[1]?.orderingJSON == data
        print("native_v2_json_column_reaches_typed_row_exactly: \(bytesExact ? "PASS" : "FAIL")")
        guard bytesExact else { return false }
        let enrichment = RemindersStoreEnrichment(paths: { [path] })
        let enriched = await enrichment.refresh(observations, generation: "ordering-fixture")
        var exact = true
        for (index, identifier) in identifiers.enumerated() {
            let record = enriched.first { $0["native_key"].string == identifier.uuidString }
            exact = exact && record?["data"]["manual_order"] == .number(Double(index)) && record?["data"]["store_coverage"]["manual_order"]["status"] == .string("complete")
        }
        print("scrubbed_native_json_ordering_enriches_exact_manual_positions: \(exact ? "PASS" : "FAIL")")
        guard exact else { return false }
        let reversed = try JSONEncoder().encode(identifiers.reversed().map { $0.uuidString })
        let reversedHex = reversed.map { String(format: "%02x", $0) }.joined()
        execute("UPDATE ZREMCDBASELIST SET ZREMINDERIDSMERGEABLEORDERING_V2_JSON=X'\(reversedHex)',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        let changed = await enrichment.refresh([], generation: "ordering-fixture")
        let current = await enrichment.apply(observations)
        let changedIdentifiers = Set(changed.filter { $0["collection"].string == "reminders.reminder" }.map { $0["native_key"].string })
        let reordered = changedIdentifiers == Set([identifiers[0].uuidString, identifiers[2].uuidString]) && identifiers.enumerated().allSatisfy { index, identifier in
            current.first { $0["native_key"].string == identifier.uuidString }?["data"]["manual_order"] == .number(Double(identifiers.count - index - 1))
        }
        print("list_only_ordering_change_reprojects_existing_reminders: \(reordered ? "PASS" : "FAIL")")
        execute("UPDATE ZREMCDBASELIST SET ZREMINDERIDSMERGEABLEORDERING_V2_JSON=X'7b2276657273696f6e223a3939397d',Z_OPT=Z_OPT+1 WHERE Z_PK=1")
        let unknown = await enrichment.refresh([], generation: "ordering-fixture")
        let hidden = identifiers.allSatisfy { identifier in
            let row = unknown.first { $0["native_key"].string == identifier.uuidString }
            return row?["data"]["manual_order"] == .null && row?["data"]["store_coverage"]["manual_order"]["status"] == .string("unsupported")
        }
        print("unknown_ordering_version_hides_previously_supported_positions: \(hidden ? "PASS" : "FAIL")")
        return reordered && hidden
    }

    private static func pureChecks(data: Data, identifiers: [UUID]) -> Bool {
        let expected = Dictionary(uniqueKeysWithValues: identifiers.enumerated().map { ($0.element, $0.offset) })
        let fixture = StoreManualOrder.decode(data) == .supported(expected)
        guard let appendedIdentifier = UUID(uuidString: "00000000-0000-4000-8000-000000000004") else { return false }
        var appended = expected
        appended[appendedIdentifier] = identifiers.count
        let narrow = StoreManualOrder.supported(appended).changedIdentifiers(from: .supported(expected)) == [appendedIdentifier]
        let stable = StoreManualOrder.supported(expected).changedIdentifiers(from: .supported(expected)) == []
        let coverage = StoreManualOrder.unsupported.changedIdentifiers(from: .supported(expected)) == nil && StoreManualOrder.supported(expected).changedIdentifiers(from: .unsupported) == nil
        print("appended_ordering_uuid_only_changes_new_position_and_coverage_transitions_invalidate: \(narrow && stable && coverage ? "PASS" : "FAIL")")
        let malformed = ["null", "{}", "[1]", "[null]", "[\"bad\"]", "{\"version\":999,\"items\":[]}", "[\"\(identifiers[0])\",\"\(identifiers[0])\"]"]
        let rejected = malformed.allSatisfy { StoreManualOrder.decode(Data($0.utf8)) == .unsupported } && StoreManualOrder.decode(nil) == .unsupported
        let empty = StoreManualOrder.decode(Data("[]".utf8)) == .supported([:])
        var seed: UInt64 = 0x123456789abcdef
        for count in 0..<4096 {
            var bytes = Data()
            for _ in 0..<(count % 257) {
                seed = seed &* 6364136223846793005 &+ 1442695040888963407
                bytes.append(UInt8(truncatingIfNeeded: seed >> 32))
            }
            _ = StoreManualOrder.decode(bytes)
        }
        for depth in [64, 256, 1024, 4096] {
            _ = StoreManualOrder.decode(Data((String(repeating: "[", count: depth) + "0" + String(repeating: "]", count: depth)).utf8))
        }
        for index in 0..<data.count {
            _ = StoreManualOrder.decode(data.prefix(index))
            var mutated = data
            mutated[index] ^= 0xff
            _ = StoreManualOrder.decode(mutated)
        }
        print("pure_ordering_fixture_malformed_unknown_version_and_4096_fuzz_inputs: \(fixture && rejected && empty ? "PASS" : "FAIL")")
        return fixture && rejected && empty && narrow && stable && coverage
    }
}
