import Foundation

@main
struct ShellProof {
    @MainActor
    static func main() async throws {
        var checks: [String: Bool] = [:]
        checks["tab_structure"] = ShellModel.tabs.map(\.rawValue) == ["settings", "approvals", "diagnostics"] && ShellModel.tabs.map(\.title) == ["Settings", "Approvals", "Diagnostics"] && Set(ShellModel.tabs.map(\.symbol)).count == 3
        var online = true
        var failPair = false
        var requests = 0
        var pairing: PairingState!
        pairing = PairingState(paired: false, pair: { code in
            guard code == "[ab-test] code", !failPair else { throw URLError(.badServerResponse) }
        }, check: {
            requests += 1
            checks["connecting_state"] = pairing.connection == .checking && pairing.busy && pairing.status == "Connecting…"
            await pairing.check()
            guard online else { throw URLError(.notConnectedToInternet) }
        })
        checks["unpaired_status"] = pairing.status == "Not paired" && !pairing.busy
        await pairing.check()
        checks["unpaired_no_request"] = requests == 0 && pairing.connection == .unpaired
        await pairing.pair("invalid")
        checks["pair_failure"] = pairing.connection == .unpaired && !pairing.busy && pairing.status == "Pairing failed. Check the code and try again."
        await pairing.pair("[ab-test] code")
        checks["paired_status"] = pairing.status == "Paired" && !pairing.busy
        await pairing.check()
        checks["connected_status"] = pairing.status == "Paired · Connected" && !pairing.busy && requests == 1
        online = false
        await pairing.check()
        checks["offline_status"] = pairing.connection == .offline && pairing.status == "Offline. Check your connection and try again." && !pairing.busy
        online = true
        await pairing.check()
        checks["reconnect_status"] = pairing.connection == .connected && pairing.status == "Paired · Connected" && requests == 3 && !pairing.busy
        failPair = true
        await pairing.pair("[ab-test] code")
        checks["failed_repair_keeps_identity"] = pairing.connection == .connected && !pairing.busy
        await pairing.check()
        checks["status_recovers_after_error"] = pairing.status == "Paired · Connected"
        let restored = PairingState(paired: true, pair: { _ in }, check: {})
        checks["existing_pair_status"] = restored.status == "Paired"
        let raw = Data("""
        [
          {"command":{"command_id":"command-b","op":"iphone.capture.photo","args":{}},"request_hash":"b","expires":102},
          {"command":{"command_id":"connector:a","op":"bridge.approval","args":{"body":"[ab-test] Code 1234 · messages:read"}},"request_hash":"a","expires":101},
          {"command":{"command_id":"expired","op":"bridge.approval","args":{"body":"expired"}},"request_hash":"e","expires":100},
          {"command":{"command_id":"command-a","op":"bridge.approval","args":{"body":""}},"request_hash":"c","expires":102}
        ]
        """.utf8)
        let jobs = try JSONDecoder().decode([PhoneJob].self, from: raw)
        let rows = ShellModel.approvals(jobs, now: 100)
        checks["approval_order_and_expiry"] = rows.map(\.id) == ["connector:a", "command-a", "command-b"]
        checks["approval_list_rendering"] = rows == [
            ShellApprovalRow(id: "connector:a", title: "Review connection", summary: "[ab-test] Code 1234 · messages:read", expires: 101),
            ShellApprovalRow(id: "command-a", title: "Review command", summary: "bridge.approval", expires: 102),
            ShellApprovalRow(id: "command-b", title: "Review command", summary: "iphone.capture.photo", expires: 102)
        ]
        checks["approval_empty_state"] = ShellModel.approvals([], now: 100).isEmpty && ShellModel.emptyApprovals == "No pending approvals"
        checks["approval_expiry_refresh"] = ShellModel.approvals(jobs, now: 102).isEmpty
        checks["approval_removal_refresh"] = ShellModel.approvals(Array(jobs.dropFirst(2)), now: 100).map(\.id) == ["command-a"]
        let suite = "ab-test-shell-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set("untouched", forKey: "unrelated")
        defaults.set(Data("invalid".utf8), forKey: "approval.malformed")
        for job in jobs { defaults.set(try JSONEncoder().encode(job), forKey: "approval." + job.command.command_id) }
        checks["stored_approval_rendering"] = ShellModel.approvals(ApprovalInbox.pending(defaults: defaults), now: 100) == rows
        ApprovalInbox.remove("connector:a", defaults: defaults)
        checks["decided_approval_removed"] = ApprovalInbox.stored("connector:a", defaults: defaults) == nil && ShellModel.approvals(ApprovalInbox.pending(defaults: defaults), now: 100).map(\.id) == ["command-a", "command-b"] && defaults.string(forKey: "unrelated") == "untouched"
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
        if checks.values.contains(false) { exit(1) }
    }
}
