import Foundation
@preconcurrency import EventKit
import SQLite3

@MainActor
struct CalendarStoreTruth {
    static func counts(tag: String, events: [EKEvent]) throws -> [String: Any] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb").path
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
            if let database { sqlite3_close(database) }
            return ["status": "unverified", "reason": "Calendar store is unavailable to AppleTruth"]
        }
        defer { sqlite3_close(database) }
        sqlite3_busy_timeout(database, 100)
        var statement: OpaquePointer?
        let sql = "SELECT i.unique_identifier,i.UUID,i.external_id,c.UUID,(SELECT count(*) FROM Attachment a WHERE a.owner_id=i.ROWID) FROM CalendarItem i JOIN Calendar c ON c.ROWID=i.calendar_id WHERE c.title=? AND substr(i.summary,1,length(?))=?"
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw TruthFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        for (index, value) in ["Bridge Test " + tag, tag, tag].enumerated() {
            guard sqlite3_bind_text(statement, Int32(index + 1), value, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) == SQLITE_OK else { throw TruthFailure.unavailable }
        }
        var count = 0, attachments = 0, calendarMatches = 0
        var columns: [String: Int] = [:]
        var externalColumns: [String: Int] = [:]
        let native = Set(events.map(\.calendarItemIdentifier))
        let external = Set(events.compactMap(\.calendarItemExternalIdentifier))
        let calendars = Set(events.map { $0.calendar.calendarIdentifier })
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { break }
            guard status == SQLITE_ROW else { throw TruthFailure.unavailable }
            count += 1
            attachments += Int(sqlite3_column_int64(statement, 4))
            for index in 0..<3 {
                if let text = sqlite3_column_text(statement, Int32(index)) {
                    let value = String(cString: text), key = ["unique_identifier", "UUID", "external_id"][index]
                    if native.contains(value) { columns[key, default: 0] += 1 }
                    if external.contains(value) { externalColumns[key, default: 0] += 1 }
                }
            }
            if let text = sqlite3_column_text(statement, 3), calendars.contains(String(cString: text)) { calendarMatches += 1 }
        }
        return ["status": "verified", "event_count": events.count, "store_count": count, "attachment_count": attachments, "calendar_identity_matches": calendarMatches, "identifier_columns": columns, "external_identifier_columns": externalColumns]
    }
}
