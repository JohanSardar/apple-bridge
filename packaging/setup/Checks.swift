import Foundation

@main
struct SetupChecks {
    static func main() throws {
        func check(_ name: String, _ passed: Bool) {
            print("\(name) passed=\(passed)")
            if !passed { exit(1) }
        }
        let permissions = SetupPermission.all
        let expected = Set(["full_disk_access", "accessibility", "contacts", "calendar", "reminders", "photos", "camera", "microphone", "screen_recording", "location", "notifications", "focus", "speech_recognition", "bluetooth", "clipboard", "automation_messages", "automation_system_events", "automation_reminders", "automation_mail", "automation_music", "automation_safari", "automation_shortcuts_events", "automation_notes", "automation_calendar"])
        check("Q1-CATALOG", Set(permissions.map(\.id)) == expected && permissions.count == expected.count && permissions.allSatisfy { !$0.why.isEmpty && !$0.fix.isEmpty && $0.settingsURL.scheme == "x-apple.systempreferences" })
        var fresh = SetupState()
        check("Q1-FRESH", fresh.step == 0 && fresh.completed.isEmpty && fresh.skipped.isEmpty)
        fresh.advance()
        for permission in permissions {
            check("Q1-GRANT-" + permission.id, fresh.label(permission.id, status: "granted") == "Allowed")
        }
        fresh.advance()
        fresh.phone = true
        fresh.advance()
        fresh.connectors = ["Claude.ai", "ChatGPT"]
        fresh.advance()
        fresh.advance()
        check("Q1-COMPLETE", fresh.step == 4 && fresh.completed == Set(0...4) && fresh.phone && fresh.connectors.count == 2)
        for permission in permissions {
            var denied = SetupState()
            denied.advance()
            check("Q1-DENY-" + permission.id, denied.label(permission.id, status: "denied") == "Needs permission" && permission.fix.contains("Other enabled features still work") && permission.fix.contains("Open Settings"))
            denied.skipped.insert(permission.id)
            check("Q1-SKIP-" + permission.id, denied.label(permission.id, status: "denied") == "Skipped" && denied.label(permission.id, status: "authorized") == "Allowed")
            for _ in 0..<8 { denied.advance() }
            check("Q1-UNSTUCK-" + permission.id, denied.step == 4)
        }
        var skipped = SetupState()
        skipped.skipped = expected
        for _ in 0..<8 { skipped.advance() }
        check("Q1-ALL-SKIPPED", skipped.step == 4 && !skipped.phone && skipped.connectors.isEmpty)
        let decoded = try JSONDecoder().decode(SetupState.self, from: JSONEncoder().encode(skipped))
        check("Q1-PERSIST", decoded.skipped == expected && decoded.completed == Set(0...4))
        for _ in 0..<8 { skipped.back() }
        check("Q1-REOPEN", skipped.step == 0)
        check("Q1-UNKNOWN", skipped.label("camera", status: nil) == "Skipped" && SetupState().label("camera", status: nil) == "Not confirmed")
        check("Q1-LIMITED", SetupState().label("photos", status: "limited") == "Limited access")
    }
}
