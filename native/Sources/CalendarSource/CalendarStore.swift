import Foundation
import SourceSupport

public struct CalendarStoreSnapshot: Sendable {
    var events: [String: [[String: JSON]]] = [:]
    var attachments: [Int: [[String: JSON]]] = [:]

    public static func ownerAddresses(path: String) throws -> [String: Set<String>] {
        let connection = try CalendarStoreConnection(path: path)
        let rows = try connection.rows("SELECT c.UUID AS calendar_uuid,hex(s.cached_external_info) AS info FROM Calendar c JOIN Store s ON s.ROWID=c.store_id WHERE s.cached_external_info IS NOT NULL")
        var result: [String: Set<String>] = [:]
        for row in rows {
            let hex = Array((row["info"]?.string ?? "").utf8)
            guard !hex.isEmpty, hex.count.isMultiple(of: 2) else { continue }
            let bytes = stride(from: 0, to: hex.count, by: 2).compactMap { UInt8(String(decoding: hex[$0..<($0 + 2)], as: UTF8.self), radix: 16) }
            guard bytes.count * 2 == hex.count,
                  let metadata = try? PropertyListSerialization.propertyList(from: Data(bytes), format: nil) as? [String: Any],
                  let owners = metadata["ownerAddresses"] as? [String] else { continue }
            let calendar = row["calendar_uuid"]?.string ?? ""
            let key = UUID(uuidString: calendar)?.uuidString ?? calendar
            for owner in owners where owner.lowercased().hasPrefix("mailto:") {
                let address = String(owner.dropFirst(7)).removingPercentEncoding?.lowercased() ?? ""
                guard address.contains("@"), !address.contains(where: { $0.isWhitespace || $0.isNewline || ";:,<>\\\"".contains($0) }) else { continue }
                result[key, default: []].insert(address)
            }
        }
        return result
    }

    public static func read(path: String) throws -> CalendarStoreSnapshot {
        let connection = try CalendarStoreConnection(path: path)
        let required: [String: Set<String>] = [
            "CalendarItem": ["ROWID", "unique_identifier", "UUID", "external_id", "calendar_id", "location_id", "self_attendee_id", "travel_time", "travel_advisory_behavior", "invitation_status", "privacy_level", "priority", "conference_url"],
            "Calendar": ["ROWID", "UUID"], "Location": ["ROWID", "title", "address", "latitude", "longitude", "radius"],
            "Participant": ["ROWID", "status"], "Conference": ["owner_id", "url"], "Attachment": ["owner_id", "file_id"],
            "AttachmentFile": ["ROWID", "UUID", "filename", "local_path", "file_size", "format"]
        ]
        for (table, fields) in required {
            let columns = Set(try connection.rows("PRAGMA table_info(" + table + ")").map { $0["name"]?.string ?? "" })
            guard fields.isSubset(of: columns) else { throw SourceFailure.unsupported }
        }
        var rows = try connection.rows("SELECT i.ROWID AS item_id,i.unique_identifier,i.UUID,i.external_id,c.UUID AS calendar_uuid,i.travel_time,i.travel_advisory_behavior,i.invitation_status,i.privacy_level,i.priority,i.conference_url,p.status AS self_status,l.title AS location_title,l.address AS location_address,l.latitude AS latitude,l.longitude AS longitude,l.radius AS radius FROM CalendarItem i JOIN Calendar c ON c.ROWID=i.calendar_id LEFT JOIN Participant p ON p.ROWID=i.self_attendee_id LEFT JOIN Location l ON l.ROWID=i.location_id")
        var conferences: [Int: Set<String>] = [:]
        for conference in try connection.rows("SELECT owner_id,url FROM Conference WHERE url IS NOT NULL") {
            guard let owner = Int(exactly: conference["owner_id"]?.number ?? 0), let link = conference["url"]?.string, !link.isEmpty else { continue }
            conferences[owner, default: []].insert(link)
        }
        for index in rows.indices where rows[index]["conference_url"] == .null {
            if let owner = Int(exactly: rows[index]["item_id"]?.number ?? 0), let links = conferences[owner], links.count == 1 { rows[index]["conference_url"] = .string(links.first!) }
        }
        var snapshot = CalendarStoreSnapshot()
        for row in rows {
            for key in Set([row["unique_identifier"]?.string ?? "", row["UUID"]?.string ?? "", row["external_id"]?.string ?? ""]) where !key.isEmpty {
                snapshot.events[key, default: []].append(row)
            }
        }
        for row in try connection.rows("SELECT a.owner_id,f.UUID,f.filename,f.local_path,f.file_size,f.format FROM Attachment a JOIN AttachmentFile f ON f.ROWID=a.file_id") {
            guard let owner = row["owner_id"], owner.number.isFinite, let key = Int(exactly: owner.number) else { continue }
            snapshot.attachments[key, default: []].append(row)
        }
        return snapshot
    }

    public func event(identifier: String, external: String, calendar: String, detached: Bool) -> [String: JSON]? {
        guard !detached else { return nil }
        for key in [identifier, external] where !key.isEmpty {
            let candidates = (events[key] ?? []).filter { row in
                let stored = row["calendar_uuid"]?.string ?? ""
                if stored == calendar { return true }
                return UUID(uuidString: stored) != nil && UUID(uuidString: stored) == UUID(uuidString: calendar)
            }
            if candidates.count > 1 { return nil }
            if candidates.count == 1 {
                var row = candidates[0]
                row["matched_by"] = .string((key == identifier ? "calendarItemIdentifier:" : "calendarItemExternalIdentifier:") + (["unique_identifier", "UUID", "external_id"].first { row[$0]?.string == key } ?? "unknown"))
                return row
            }
        }
        return nil
    }
}
