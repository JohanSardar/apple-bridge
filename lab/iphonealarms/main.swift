import Foundation

@MainActor
final class FakeAlarmAdapter: AlarmAdapter {
    var alarms: [UUID: BridgeAlarm] = [:]
    var allowed = true
    func authorize() async throws {
        if !allowed { throw AlarmCommandError.permission }
    }
    func list() -> [BridgeAlarm] { Array(alarms.values) }
    func schedule(_ alarm: BridgeAlarm) -> BridgeAlarm {
        alarms[alarm.id] = alarm
        return alarm
    }
    func cancel(_ id: UUID) { alarms.removeValue(forKey: id) }
}

@main
struct AlarmChecks {
    @MainActor
    static func main() async throws {
        let fake = FakeAlarmAdapter()
        let commands = AlarmCommands(adapter: fake)
        func run(_ op: String, _ args: String) async throws -> Data {
            try await commands.execute("iphone." + op, Data(args.utf8))
        }
        let fire = Date().timeIntervalSince1970 + 60
        let created = try JSONDecoder().decode(BridgeAlarm.self, from: await run("alarm.create", "{\"fire_at\":\(fire),\"title\":\"[ab-test]\",\"snooze_seconds\":60}"))
        let updated = try JSONDecoder().decode(BridgeAlarm.self, from: await run("alarm.update", "{\"id\":\"\(created.id)\",\"fire_at\":\(fire+60),\"snooze_seconds\":120}"))
        precondition(updated.fire_at == fire + 60 && updated.snooze_seconds == 120)
        let listed = try JSONDecoder().decode([BridgeAlarm].self, from: await run("alarm.list", "{}"))
        precondition(listed.count == 1 && listed[0].id == created.id)
        _ = try await run("alarm.cancel", "{\"id\":\"\(created.id)\"}")
        let timer = try JSONDecoder().decode(BridgeAlarm.self, from: await run("timer.start", "{\"duration_seconds\":60}"))
        precondition(timer.duration_seconds == 60 && timer.fire_at == nil)
        _ = try await run("timer.cancel", "{\"id\":\"\(timer.id)\"}")
        precondition(fake.alarms.isEmpty)
        fake.allowed = false
        do {
            _ = try await run("alarm.create", "{\"fire_at\":\(fire)}")
            fatalError("permission was not enforced")
        } catch AlarmCommandError.permission {}
        print("passed")
    }
}
