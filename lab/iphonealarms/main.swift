import Foundation

@MainActor
final class FakeAlarmAdapter: AlarmAdapter {
    var alarms: [UUID: BridgeAlarm] = [:]
    var allowed = true
    var scheduleFails = false
    var cancelFails = false
    func authorize() async throws {
        if !allowed { throw AlarmCommandError.permission }
    }
    func list() -> [BridgeAlarm] { Array(alarms.values) }
    func schedule(_ alarm: BridgeAlarm) throws -> BridgeAlarm {
        alarms[alarm.id] = alarm
        if scheduleFails { throw CocoaError(.fileWriteUnknown) }
        return alarm
    }
    func cancel(_ id: UUID) throws {
        if cancelFails { throw CocoaError(.fileReadUnknown) }
        alarms.removeValue(forKey: id)
    }
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
        let qualification = BridgeAlarm(id: UUID(), fire_at: fire, duration_seconds: nil, snooze_seconds: 60, title: "[ab-test] qualification", state: "scheduled")
        var checks: [String: Bool] = [:]
        try await AlarmQualification.run(qualification, adapter: fake) { checks[$0] = $1 }
        precondition(checks["created_and_read"] == true && checks["deleted_and_absent"] == true && fake.alarms.isEmpty)
        fake.scheduleFails = true
        for cancelFails in [false, true] {
            fake.cancelFails = cancelFails
            do {
                try await AlarmQualification.run(qualification, adapter: fake) { checks[$0] = $1 }
                fatalError("schedule failure was hidden")
            } catch { precondition((error as NSError).code == CocoaError.fileWriteUnknown.rawValue) }
            precondition(checks["cleanup_after_failure"] == !cancelFails)
        }
        fake.cancelFails = false
        try fake.cancel(qualification.id)
        fake.allowed = false
        do {
            _ = try await run("alarm.create", "{\"fire_at\":\(fire)}")
            fatalError("permission was not enforced")
        } catch AlarmCommandError.permission {}
        print("passed")
    }
}
