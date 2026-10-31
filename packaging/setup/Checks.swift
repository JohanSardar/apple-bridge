import Foundation

@main
struct SetupChecks {
    @MainActor static func main() async throws {
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
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-setup-model-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: home) }
        let model = SetupModel(home: home)
        var copied: [String] = []
        var opened: [URL] = []
        var installed = false
        model.copyURL = { copied.append($0) }
        model.openURL = { opened.append($0) }
        model.install = { installed = true }
        model.syncStatus = { installed }
        model.runCommand = { arguments, path in
            if arguments.first == "install-credentials" { return Data() }
            if arguments.first == "remote" {
                let remote = URL(fileURLWithPath: path).appendingPathComponent("remote")
                try? FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
                try? Data(#"{"state":"on","url":"https://bridge.example.test"}"#.utf8).write(to: remote.appendingPathComponent("status.json"))
                return Data()
            }
            if arguments == ["pair", "--iphone", "--via-remote"] { return Data("[ab-test] one-time-pairing-code\nQR text\n".utf8) }
            return nil
        }
        func settled(_ key: String) async -> Bool {
            for _ in 0..<200 {
                if !model.busy.contains(key) { return true }
                try? await Task.sleep(for: .milliseconds(10))
            }
            return false
        }
        await model.refresh()
        check("Q1-NO-PROMPTS", model.permissions.requests.isEmpty)
        model.enableSync()
        let syncSettled = await settled("sync")
        check("Q1-SYNC", syncSettled && installed && model.syncEnabled)
        model.move(true)
        for permission in permissions {
            model.request(permission)
            let finished = await settled(permission.id)
            check("Q1-ROW-" + permission.id, finished && model.permissions.requests.last == permission.id && model.state.label(permission.id, status: model.statuses[permission.id]) == "Allowed")
        }
        check("Q1-ONLY-CLICKED-ROWS", model.permissions.requests == permissions.map(\.id))
        model.move(true)
        model.enableRemote()
        let remoteSettled = await settled("remote")
        check("Q1-REMOTE", remoteSettled && model.remoteReady)
        model.pair()
        let pairingSettled = await settled("pair")
        check("Q1-QR", pairingSettled && model.qr != nil && model.pairingExpiry != nil)
        let credentials = #"{"principals":[{"kind":"replica","iphone_signing_key":"[ab-test]"},{"kind":"agent","name":"Claude"},{"kind":"agent","name":"ChatGPT"}]}"#
        try Data(credentials.utf8).write(to: home.appendingPathComponent("credentials.json"))
        await model.refresh()
        model.move(true)
        model.connect("Claude.ai")
        model.connect("ChatGPT")
        check("Q1-CONNECT", opened.count == 2 && copied == Array(repeating: "https://bridge.example.test/mcp", count: 2) && model.message.contains("four-digit code"))
        model.move(true)
        model.state.advance()
        check("Q1-FLOW", model.state.step == 4 && model.state.completed == Set(0...4) && model.state.phone && model.state.connectors.count == 2)
        model.pairingExpiry = Date().addingTimeInterval(-1)
        await model.refresh()
        check("Q1-EXPIRED-QR", model.qr == nil && model.message.contains("expired"))
        model.runCommand = { _, _ in nil }
        model.pair()
        let failedPairSettled = await settled("pair")
        check("Q1-PAIR-FAILURE", failedPairSettled && model.message.contains("Enable Remote") && model.message.contains("continue"))
        model.remoteReady = false
        model.connect("ChatGPT")
        check("Q1-CONNECT-FAILURE", model.message.contains("Enable Remote") && opened.count == 2)
        model.permissions.denied = "camera"
        model.request(permissions.first { $0.id == "camera" }!)
        let deniedSettled = await settled("camera")
        check("Q1-DENIED-ROW", deniedSettled && model.state.label("camera", status: model.statuses["camera"]) == "Needs permission")
        model.skip("camera")
        model.state.step = 1
        model.move(true)
        check("Q1-DENIED-CONTINUE", model.state.step == 2 && model.state.skipped.contains("camera"))
        model.save()
        let resumed = SetupModel(home: home)
        check("Q1-MODEL-RESUME", resumed.state.step == 2 && resumed.state.skipped.contains("camera"))
    }
}
