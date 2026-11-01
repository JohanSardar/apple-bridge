import Foundation
import SQLite3

public enum StoreScalar: Equatable, Sendable {
    case integer(Int64), real(Double), text(String)
}

public struct StoreIdentity: Equatable, Sendable {
    public let key: Int64
    public let entity: Int64
    public let version: Int64?
    public let uuid: UUID?
    public let textIdentifier: String?
}

public struct StoreReminderRow: Equatable, Sendable {
    public let identity: StoreIdentity
    public let calendarItemIdentifier: String?
    public let externalIdentifier: String?
    public let flagged: Bool?
    public let parent: Int64?
    public let list: Int64?
    public let created: Double?
    public let earlyReminder: Data?
}

public struct StoreListRow: Equatable, Sendable {
    public let identity: StoreIdentity
    public let name: String?
    public let emblem: String?
    public let isGroup: Bool?
    public let parent: Int64?
    public let alternateParent: Int64?
    public let displayOrder: Double?
    public let pinned: Bool?
    public let pinnedDate: Double?
    public let sortingStyle: StoreScalar?
    public let groceries: Bool?
    public let groceryLocale: String?
    public let sharingStatus: Int64?
    public let sharedOwner: String?
    public let smartType: StoreScalar?
    public let filter: Data?
    public let sectionOrder: Data?
    public let sectionMembership: Data?
    public let ordering: Data?
    public let orderingJSON: Data?
}

public struct StoreSectionRow: Equatable, Sendable {
    public let identity: StoreIdentity
    public let name: String?
    public let list: Int64?
    public let smartList: Int64?
    public let created: Double?
}

public struct StoreObjectRow: Equatable, Sendable {
    public let identity: StoreIdentity
    public let reminder: Int64?
    public let reminder1: Int64?
    public let reminder2: Int64?
    public let reminder3: Int64?
    public let reminder4: Int64?
    public let reminder5: Int64?
    public let hashtagLabel: Int64?
    public let list: Int64?
}

public struct StoreLabelRow: Equatable, Sendable {
    public let key: Int64
    public let name: String?
}

public struct TypedStoreSnapshot: Equatable, Sendable {
    public var reminders: [Int64: StoreReminderRow] = [:]
    public var lists: [Int64: StoreListRow] = [:]
    public var sections: [Int64: StoreSectionRow] = [:]
    public var objects: [Int64: StoreObjectRow] = [:]
    public var labels: [Int64: StoreLabelRow] = [:]
}

struct StoreColumns {
    let statement: OpaquePointer

    func integer(_ index: Int32) -> Int64? {
        sqlite3_column_type(statement, index) == SQLITE_INTEGER ? sqlite3_column_int64(statement, index) : nil
    }

    func number(_ index: Int32) -> Double? {
        [SQLITE_INTEGER, SQLITE_FLOAT].contains(sqlite3_column_type(statement, index)) ? sqlite3_column_double(statement, index) : nil
    }

    func bool(_ index: Int32) -> Bool? {
        guard let value = integer(index), value == 0 || value == 1 else { return nil }
        return value == 1
    }

    func scalar(_ index: Int32) -> StoreScalar? {
        switch sqlite3_column_type(statement, index) {
        case SQLITE_INTEGER: return integer(index).map(StoreScalar.integer)
        case SQLITE_FLOAT: return number(index).map(StoreScalar.real)
        case SQLITE_TEXT: return text(index).map(StoreScalar.text)
        default: return nil
        }
    }

    func text(_ index: Int32) -> String? {
        guard sqlite3_column_type(statement, index) == SQLITE_TEXT, let bytes = sqlite3_column_text(statement, index) else { return nil }
        return String(decoding: UnsafeBufferPointer(start: bytes, count: Int(sqlite3_column_bytes(statement, index))), as: UTF8.self)
    }

    func data(_ index: Int32) -> Data? {
        guard sqlite3_column_type(statement, index) != SQLITE_NULL else { return nil }
        guard let bytes = sqlite3_column_blob(statement, index) else { return Data() }
        return Data(bytes: bytes, count: Int(sqlite3_column_bytes(statement, index)))
    }

    var identity: StoreIdentity {
        let identifier: UUID?
        if sqlite3_column_type(statement, 3) == SQLITE_BLOB, sqlite3_column_bytes(statement, 3) == 16, let bytes = sqlite3_column_blob(statement, 3) {
            identifier = UUID(uuid: bytes.loadUnaligned(as: uuid_t.self))
        } else { identifier = nil }
        return StoreIdentity(key: integer(0)!, entity: integer(1)!, version: integer(2), uuid: identifier, textIdentifier: sqlite3_column_type(statement, 3) == SQLITE_TEXT ? text(3) : nil)
    }
}
