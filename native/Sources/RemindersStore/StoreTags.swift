import Foundation
import SourceSupport

public enum StoreTags {
    public static func names(for reminder: Int64, snapshot: TypedStoreSnapshot) -> [String]? {
        var names: Set<String> = []
        for object in snapshot.objects.values where object.hashtagLabel != nil {
            let owners = Set([object.reminder, object.reminder1, object.reminder2, object.reminder3, object.reminder4, object.reminder5].compactMap { $0 }.filter { $0 != 0 })
            guard owners.contains(reminder) else { continue }
            guard owners.count == 1, let label = object.hashtagLabel, let name = snapshot.labels[label]?.name, !name.isEmpty else { return nil }
            names.insert(name)
        }
        return names.sorted()
    }
}
