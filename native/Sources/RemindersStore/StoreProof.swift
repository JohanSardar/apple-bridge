#if LAB
import Foundation
import SourceSupport

public enum RemindersStoreProof {
    public static func run(tag: String, itemIdentifier: String, listIdentifier: String) throws -> JSON {
        guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !itemIdentifier.isEmpty, !listIdentifier.isEmpty else { throw SourceFailure.invalid }
        let start = ContinuousClock.now
        let snapshots = try RemindersStore.paths().map { try RemindersStore(path: $0).read() }
        let matches = snapshots.flatMap { snapshot in
            snapshot.reminders.filter { $0["ZTITLE"]?.string == tag + " identity" }.map { (snapshot, $0) }
        }
        guard matches.count == 1, let (snapshot, reminder) = matches.first else { throw SourceFailure.conflict }
        let listKey = Int(reminder["ZLIST"]?.number ?? 0)
        guard let list = snapshot.lists.first(where: { Int($0["Z_PK"]?.number ?? 0) == listKey }) else { throw SourceFailure.corrupt }
        let parentKeys = Dictionary(uniqueKeysWithValues: snapshot.reminders.map { (Int($0["Z_PK"]?.number ?? 0), $0["ZDACALENDARITEMUNIQUEIDENTIFIER"]?.string ?? "") })
        let listKeys = Dictionary(uniqueKeysWithValues: snapshot.lists.map { (Int($0["Z_PK"]?.number ?? 0), $0["ZIDENTIFIER"]?.string ?? "") })
        let duration = start.duration(to: .now).components
        let milliseconds = Double(duration.seconds) * 1000 + Double(duration.attoseconds) / 1e15
        return .object([
            "da_equals_eventkit": .bool(reminder["ZDACALENDARITEMUNIQUEIDENTIFIER"]?.string == itemIdentifier),
            "uuid_equals_eventkit": .bool(reminder["ZIDENTIFIER"]?.string == itemIdentifier),
            "external_equals_eventkit": .bool(reminder["ZEXTERNALIDENTIFIER"]?.string == itemIdentifier),
            "list_uuid_equals_eventkit": .bool(list["ZIDENTIFIER"]?.string == listIdentifier),
            "reminder_fields": snapshot.reminderFields(reminder, parentKeys: parentKeys),
            "list_fields": snapshot.listFields(list, listKeys: listKeys),
            "store_read_ms": .number(milliseconds)
        ])
    }
}
#endif
