import Foundation
import SQLite3
import IOKit

public enum UsageHistory {
    public static func blocked() -> [DateInterval] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Knowledge/knowledgeC.db").path
        var database: OpaquePointer?, statement: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if let database { sqlite3_close(database) }; return [] }
        defer { sqlite3_close(database) }
        let service = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"))
        let identifier = service == 0 ? "" : (IORegistryEntryCreateCFProperty(service, "IOPlatformUUID" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? String ?? "")
        if service != 0 { IOObjectRelease(service) }
        let sql = "SELECT o.ZSTARTDATE,o.ZENDDATE FROM ZOBJECT o LEFT JOIN ZSOURCE s ON s.Z_PK=o.ZSOURCE WHERE ((o.ZSTREAMNAME='/device/isLocked' AND o.ZVALUEINTEGER=1) OR (o.ZSTREAMNAME='/display/isBacklit' AND o.ZVALUEINTEGER=0)) AND (o.ZSOURCE IS NULL OR s.ZDEVICEID=?)"
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(statement) }
        sqlite3_bind_text(statement, 1, identifier, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
        var result: [DateInterval] = []
        var step = sqlite3_step(statement)
        while step == SQLITE_ROW {
            let start = sqlite3_column_double(statement, 0) + 978307200, end = sqlite3_column_double(statement, 1) + 978307200
            if start.isFinite, end.isFinite, end > start, start > 1.6e9, end < Date().timeIntervalSince1970 + 60 { result.append(DateInterval(start: Date(timeIntervalSince1970: start), end: Date(timeIntervalSince1970: end))) }
            step = sqlite3_step(statement)
        }
        return step == SQLITE_DONE ? result : []
    }
}
