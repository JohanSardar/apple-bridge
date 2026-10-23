import Foundation
import SourceSupport

struct NotesChangeCheckpoint {
    let schema: String
    let objects: [Int: [String: JSON]]
    let bodies: [Int: [String: JSON]]
}

struct NotesChangeScan {
    let snapshot: NotesSnapshot
    let removed: Set<String>
    let checkpoint: NotesChangeCheckpoint
    let changedNotes: Int
    let bootstrap: Bool
}

@MainActor
final class NotesChangeStore {
    private let path: String
    private var checkpoint: NotesChangeCheckpoint?
    init(path: String) { self.path = path }
    func accept(_ scan: NotesChangeScan) { checkpoint = scan.checkpoint }
    func scan() throws -> NotesChangeScan {
        let db = try NotesDatabase(path: path)
        let columns = try db.columns("ZICCLOUDSYNCINGOBJECT")
        guard columns.contains("Z_OPT"), try db.columns("ZICNOTEDATA").contains("Z_OPT") else { throw SourceFailure.unsupported }
        let entities = try db.query("SELECT Z_ENT,Z_NAME FROM Z_PRIMARYKEY")
        guard let noteEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICNote" })?["Z_ENT"]?.number,
              let folderEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICFolder" })?["Z_ENT"]?.number,
              let accountEntity = entities.first(where: { $0["Z_NAME"]?.string == "ICAccount" })?["Z_ENT"]?.number,
              let uuid = try db.query("SELECT Z_UUID FROM Z_METADATA").first?["Z_UUID"]?.string else { throw SourceFailure.unsupported }
        let schema = uuid + ":" + String(try db.query("PRAGMA schema_version").first?["schema_version"]?.number ?? 0)
        let fields = ["Z_PK", "Z_ENT", "Z_OPT", "ZIDENTIFIER", "ZNOTEDATA", "ZFOLDER", "ZPARENT", "ZACCOUNT8", "ZNOTE", "ZNOTE1", "ZPARENTATTACHMENT", "ZMEDIA", "ZMARKEDFORDELETION"].filter { columns.contains($0) }
        let rows = try db.query("SELECT " + fields.joined(separator: ",") + " FROM ZICCLOUDSYNCINGOBJECT")
        let objects = Dictionary(uniqueKeysWithValues: rows.map { (Int($0["Z_PK"]?.number ?? 0), $0) })
        let bodies = Dictionary(uniqueKeysWithValues: try db.query("SELECT Z_PK,Z_OPT FROM ZICNOTEDATA").map { (Int($0["Z_PK"]?.number ?? 0), $0) })
        let previous = checkpoint?.schema == schema ? checkpoint : nil
        var changed = Set<Int>(), removed = Set<String>()
        let previousObjects = previous?.objects ?? [:]
        let changedObjects = Set(objects.keys).union(previousObjects.keys).filter { objects[$0] != previousObjects[$0] }
        let previousBodies = previous?.bodies ?? [:]
        let changedBodies = Set(bodies.keys).union(previousBodies.keys).filter { bodies[$0] != previousBodies[$0] }
        let all = objects.merging(previousObjects) { current, _ in current }
        func owner(_ id: Int, historical: Bool = false, seen: Set<Int> = []) -> Int? {
            let rows = historical ? previousObjects : all
            guard !seen.contains(id), let row = rows[id], seen.count < 32 else { return nil }
            if row["Z_ENT"]?.number == noteEntity { return id }
            for field in ["ZNOTE", "ZNOTE1", "ZPARENTATTACHMENT"] {
                let parent = Int(row[field]?.number ?? 0)
                if parent > 0, let result = owner(parent, historical: historical, seen: seen.union([id])) { return result }
            }
            return nil
        }
        var mediaOwners: [Int: Set<Int>] = [:]
        for rows in [all, previousObjects] {
            for (pk, row) in rows {
                if let media = row["ZMEDIA"]?.number, media > 0 { mediaOwners[Int(media), default: []].insert(pk) }
            }
        }
        for id in changedObjects {
            if let old = previousObjects[id], old["ZIDENTIFIER"] != objects[id]?["ZIDENTIFIER"] {
                let collection = old["Z_ENT"]?.number == noteEntity ? "notes.note" : old["Z_ENT"]?.number == folderEntity ? "notes.folder" : ""
                if !collection.isEmpty, let key = old["ZIDENTIFIER"]?.string { removed.insert(collection + "/" + key) }
            }
            for historical in [false, true] {
                if let note = owner(id, historical: historical) { changed.insert(note) }
                for pk in mediaOwners[id, default: []] {
                    if let note = owner(pk, historical: historical) { changed.insert(note) }
                }
            }
            if let row = all[id], row["Z_ENT"]?.number == folderEntity || row["Z_ENT"]?.number == accountEntity {
                var affected = Set([id]), expanded = true
                while expanded {
                    expanded = false
                    for (pk, candidate) in all where candidate["Z_ENT"]?.number == folderEntity {
                        if affected.contains(Int(candidate["ZPARENT"]?.number ?? 0)) || affected.contains(Int(candidate["ZACCOUNT8"]?.number ?? 0)) {
                            if affected.insert(pk).inserted { expanded = true }
                        }
                    }
                }
                for (pk, note) in all where note["Z_ENT"]?.number == noteEntity && affected.contains(Int(note["ZFOLDER"]?.number ?? 0)) { changed.insert(pk) }
            }
        }
        for (pk, note) in all where note["Z_ENT"]?.number == noteEntity && changedBodies.contains(Int(note["ZNOTEDATA"]?.number ?? 0)) { changed.insert(pk) }
        var keys = Set<String>()
        for pk in changed {
            if let row = objects[pk], (row["ZMARKEDFORDELETION"]?.number ?? 0) == 0 { keys.insert(row["ZIDENTIFIER"]?.string ?? "") }
            else if let row = previousObjects[pk], let key = row["ZIDENTIFIER"]?.string { removed.insert("notes.note/" + key) }
        }
        for pk in changedObjects {
            if objects[pk] == nil || (objects[pk]?["ZMARKEDFORDELETION"]?.number ?? 0) != 0,
               let old = previousObjects[pk], old["Z_ENT"]?.number == folderEntity, let key = old["ZIDENTIFIER"]?.string { removed.insert("notes.folder/" + key) }
        }
        let snapshot = try NotesStore(path: path).read(keys: previous == nil ? nil : keys, metadataOnly: true, database: db)
        let projected = Set(snapshot.observations.filter { $0["collection"].string == "notes.note" }.map { $0["native_key"].string })
        removed.formUnion(keys.subtracting(projected).map { "notes.note/" + $0 })
        removed.subtract(snapshot.observations.map { $0["collection"].string + "/" + $0["native_key"].string })
        return NotesChangeScan(snapshot: snapshot, removed: removed, checkpoint: NotesChangeCheckpoint(schema: schema, objects: objects, bodies: bodies), changedNotes: projected.count, bootstrap: previous == nil)
    }
}
