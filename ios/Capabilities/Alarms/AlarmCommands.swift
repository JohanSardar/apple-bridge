import Foundation

struct AlarmRequest: Codable {
    var id: UUID?
    var fire_at: Double?
    var duration_seconds: Double?
    var snooze_seconds: Double?
    var title: String?
}

struct BridgeAlarm: Codable {
    var id: UUID
    var fire_at: Double?
    var duration_seconds: Double?
    var snooze_seconds: Double
    var title: String
    var state: String
}

@MainActor
protocol AlarmAdapter {
    func authorize() async throws
    func list() throws -> [BridgeAlarm]
    func schedule(_ alarm: BridgeAlarm) async throws -> BridgeAlarm
    func cancel(_ id: UUID) throws
}

@MainActor
struct AlarmCommands {
    let adapter: any AlarmAdapter

    func execute(_ operation: String, _ data: Data) async throws -> Data {
        let request = try JSONDecoder().decode(AlarmRequest.self, from: data)
        try await adapter.authorize()
        if operation == "iphone.alarm.list" {
            return try JSONEncoder().encode(adapter.list())
        }
        if operation.hasSuffix(".cancel") {
            guard let id = request.id else { throw AlarmCommandError.invalid }
            try adapter.cancel(id)
            return Data("{\"cancelled\":true}".utf8)
        }
        guard ["iphone.alarm.create", "iphone.alarm.update", "iphone.timer.start"].contains(operation) else {
            throw AlarmCommandError.invalid
        }
        let timer = operation == "iphone.timer.start"
        if operation == "iphone.alarm.update" {
            guard let id = request.id, try adapter.list().contains(where: { $0.id == id }) else {
                throw AlarmCommandError.invalid
            }
        }
        guard timer ? (request.duration_seconds ?? 0) > 0 : (request.fire_at ?? 0) > Date().timeIntervalSince1970,
              (request.snooze_seconds ?? 540) > 0 else { throw AlarmCommandError.invalid }
        let alarm = BridgeAlarm(id: request.id ?? UUID(), fire_at: timer ? nil : request.fire_at,
                                duration_seconds: timer ? request.duration_seconds : nil,
                                snooze_seconds: request.snooze_seconds ?? 540,
                                title: request.title ?? "Apple Bridge", state: "scheduled")
        return try JSONEncoder().encode(await adapter.schedule(alarm))
    }
}

enum AlarmCommandError: Error {
    case invalid
    case permission
}

@MainActor
enum AlarmQualification {
    static func run(_ alarm: BridgeAlarm, adapter: any AlarmAdapter, check: (String, Bool) -> Void) async throws {
        do {
            _ = try await adapter.schedule(alarm)
            check("created_and_read", try adapter.list().contains { $0.id == alarm.id && $0.title == alarm.title })
        } catch {
            do {
                if try adapter.list().contains(where: { $0.id == alarm.id }) { try adapter.cancel(alarm.id) }
                check("cleanup_after_failure", try !adapter.list().contains { $0.id == alarm.id })
            } catch { check("cleanup_after_failure", false) }
            throw error
        }
        try adapter.cancel(alarm.id)
        check("deleted_and_absent", try !adapter.list().contains { $0.id == alarm.id })
    }
}
