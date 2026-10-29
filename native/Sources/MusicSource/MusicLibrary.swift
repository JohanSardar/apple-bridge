import Foundation
import SourceSupport

extension MusicPlayer {
    func library() throws -> [JSON] {
        let data = try script("""
        if (count tracks of library playlist 1) > 500 or (count user playlists) > 50 then error "capacity"
        set songs to {}
        repeat with t in tracks of library playlist 1
            set end of songs to {persistent ID of t, name of t, artist of t, album of t, favorited of t}
        end repeat
        set lists to {}
        set memberships to 0
        repeat with p in user playlists
            set memberships to memberships + (count tracks of p)
            if memberships > 1000 then error "capacity"
            set ids to {}
            repeat with t in tracks of p
                set end of ids to persistent ID of t
            end repeat
            set end of lists to {persistent ID of p, name of p, ids}
        end repeat
        return {songs, lists}
        """)
        guard data.numberOfItems == 2, let songs = data.atIndex(1), let lists = data.atIndex(2), songs.numberOfItems <= 500, lists.numberOfItems <= 50 else { throw SourceFailure.corrupt }
        var rows: [JSON] = []
        var ids = Set<String>()
        for index in 0..<songs.numberOfItems {
            guard let row = songs.atIndex(index + 1), row.numberOfItems == 5, let id = row.atIndex(1)?.stringValue?.uppercased(), ids.insert(id).inserted else { throw SourceFailure.corrupt }
            rows.append(.object(["kind": .string("upsert"), "collection": .string("music.library_item"), "native_key": .string(id), "data": .object(["name": .string(row.atIndex(2)?.stringValue ?? ""), "artist": .string(row.atIndex(3)?.stringValue ?? ""), "album": .string(row.atIndex(4)?.stringValue ?? ""), "favorite": .bool(row.atIndex(5)?.booleanValue ?? false)])]))
        }
        var total = 0
        for index in 0..<lists.numberOfItems {
            guard let row = lists.atIndex(index + 1), row.numberOfItems == 3, let id = row.atIndex(1)?.stringValue?.uppercased(), let members = row.atIndex(3) else { throw SourceFailure.corrupt }
            total += members.numberOfItems
            guard total <= 1000 else { throw SourceFailure.invalid }
            var items: [JSON] = []
            for member in 0..<members.numberOfItems {
                guard let key = members.atIndex(member + 1)?.stringValue?.uppercased(), ids.contains(key) else { throw SourceFailure.corrupt }
                items.append(.object(["$ref": .object(["collection": .string("music.library_item"), "native_key": .string(key)])]))
            }
            rows.append(.object(["kind": .string("upsert"), "collection": .string("music.playlist"), "native_key": .string(id), "data": .object(["name": .string(row.atIndex(2)?.stringValue ?? ""), "items": .array(items), "count": .integer(UInt64(items.count))])]))
        }
        return rows
    }
    func publishLibrary(_ rows: [JSON]) async throws {
        guard !publishing, let sink else { throw SourceFailure.unavailable }
        publishing = true
        defer { publishing = false }
        let started = timestamp()
        let stamped = rows.map { row -> JSON in var value = row.object; value["scoped_alias"] = .string("music/" + row["collection"].string + "/" + row["native_key"].string); value["observed_at"] = .string(started); return .object(value) }
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: stamped, scanID: UUID().uuidString, scanStartedAt: started, scope: ["music.library_item", "music.playlist"], completionEvidence: "complete", blobHandles: [:], live: false)
    }
}
