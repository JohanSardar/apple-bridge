import Foundation
import SourceSupport

public struct RemindersStoreSnapshot: Sendable {
    public let reminders: [[String: JSON]]
    public let lists: [[String: JSON]]
    public let sections: [[String: JSON]]
    public let objects: [[String: JSON]]
    public let labels: [[String: JSON]]
    public let entities: [Int: String]
}

public struct RemindersStore: Sendable {
    public let path: String

    public init(path: String) { self.path = path }

    public static func paths(home: URL = FileManager.default.homeDirectoryForCurrentUser) throws -> [String] {
        let stores = home.appendingPathComponent("Library/Group Containers/group.com.apple.reminders/Container_v1/Stores", isDirectory: true)
        return try FileManager.default.contentsOfDirectory(at: stores, includingPropertiesForKeys: [.isRegularFileKey]).filter {
            $0.lastPathComponent.hasPrefix("Data-") && $0.pathExtension == "sqlite" && (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
        }.map(\.path).sorted()
    }

    public func read() throws -> RemindersStoreSnapshot {
        let copy = try StoreCopy(path: path)
        let keys = try copy.rows("SELECT Z_ENT,Z_NAME,Z_SUPER FROM Z_PRIMARYKEY")
        var entities: [Int: String] = [:]
        for key in keys {
            let number = Int(key["Z_ENT"]?.number ?? 0)
            guard number > 0, let name = key["Z_NAME"], !name.string.isEmpty, entities[number] == nil else { throw SourceFailure.corrupt }
            entities[number] = name.string
        }
        guard entities.values.contains("REMCDReminder"), entities.values.contains("REMCDList"), entities.values.contains("REMCDSmartList") else { throw SourceFailure.unsupported }
        func rows(_ table: String, ancestor: String, columns: [String]) throws -> [[String: JSON]] {
            let root = keys.first { $0["Z_NAME"]?.string == ancestor }?["Z_ENT"]?.number
            guard let root else { throw SourceFailure.unsupported }
            var descendants: Set<Int> = [Int(root)]
            for _ in 0..<keys.count {
                let next = descendants.union(keys.filter { descendants.contains(Int($0["Z_SUPER"]?.number ?? 0)) }.map { Int($0["Z_ENT"]?.number ?? 0) })
                if next == descendants { break }
                descendants = next
            }
            let schema = Set(try copy.rows("PRAGMA table_info(" + table + ")").compactMap { $0["name"]?.string })
            let common = ["Z_PK", "Z_ENT", "ZMARKEDFORDELETION", "ZCKNEEDSTOBEFETCHEDFROMCLOUD", "ZCKNEEDSINITIALFETCHFROMCLOUD", "ZIDENTIFIER"]
            guard Set(common).isSubset(of: schema) else { throw SourceFailure.unsupported }
            let selected = (common + columns).filter { schema.contains($0) }.joined(separator: ",")
            return try copy.rows("SELECT " + selected + " FROM " + table + " ORDER BY Z_PK").filter { row in
                descendants.contains(Int(row["Z_ENT"]?.number ?? 0)) && row["ZMARKEDFORDELETION"]?.number == 0 && (row["ZCKNEEDSTOBEFETCHEDFROMCLOUD"]?.number ?? 0) == 0 && (row["ZCKNEEDSINITIALFETCHFROMCLOUD"]?.number ?? 0) == 0
            }
        }
        return RemindersStoreSnapshot(
            reminders: try rows("ZREMCDREMINDER", ancestor: "REMCDReminder", columns: ["ZTITLE", "ZDACALENDARITEMUNIQUEIDENTIFIER", "ZEXTERNALIDENTIFIER", "ZFLAGGED", "ZPARENTREMINDER", "ZLIST", "ZICSDISPLAYORDER", "ZCREATIONDATE", "ZDUEDATEDELTAALERTSDATA", "ZCONTACTHANDLES"]),
            lists: try rows("ZREMCDBASELIST", ancestor: "REMCDBaseList", columns: ["ZNAME", "ZBADGEEMBLEM", "ZCOLOR", "ZISGROUP", "ZPARENTLIST", "ZPARENTLIST1", "ZDADISPLAYORDER", "ZISPINNEDBYCURRENTUSER", "ZPINNEDDATE", "ZSORTINGSTYLE", "ZSHOULDCATEGORIZEGROCERYITEMS", "ZGROCERYLOCALEID", "ZSHARINGSTATUS", "ZSHAREDOWNERNAME", "ZSMARTLISTTYPE", "ZFILTERDATA", "ZSECTIONIDSORDERINGASDATA", "ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA", "ZREMINDERIDSMERGEABLEORDERING", "ZREMINDERIDSMERGEABLEORDERING_V2_JSON"]),
            sections: try rows("ZREMCDBASESECTION", ancestor: "REMCDBaseSection", columns: ["ZDISPLAYNAME", "ZLIST", "ZSMARTLIST", "ZCREATIONDATE"]),
            objects: try rows("ZREMCDOBJECT", ancestor: "REMCDObject", columns: ["ZREMINDER", "ZREMINDER1", "ZREMINDER2", "ZREMINDER3", "ZREMINDER4", "ZREMINDER5", "ZHASHTAGLABEL", "ZUTI", "ZFILENAME", "ZFILESIZE", "ZWIDTH", "ZHEIGHT", "ZSHA512SUM", "ZALARM", "ZTRIGGER", "ZTIMEINTERVAL", "ZDATECOMPONENTSDATA", "ZLATITUDE", "ZLONGITUDE", "ZRADIUS", "ZPROXIMITY", "ZASSIGNEE", "ZORIGINATOR", "ZASSIGNEDDATE", "ZFREQUENCY", "ZINTERVAL", "ZOCCURRENCECOUNT", "ZENDDATE", "ZFIRSTDAYOFTHEWEEK"]),
            labels: try copy.rows("SELECT * FROM ZREMCDHASHTAGLABEL ORDER BY Z_PK"),
            entities: entities
        )
    }
}
