import Foundation

@main
struct SetupChecks {
    static func main() throws {
        func check(_ name: String, _ passed: Bool) {
            print("\(name) passed=\(passed)")
            if !passed { exit(1) }
        }
        let known = Set(SetupPermission.all.map(\.id))
        check("Q1-CATALOG", known.count == SetupPermission.all.count && SetupService.all.allSatisfy { Set($0.permissions).isSubset(of: known) })
        check("Q1-EVERY-PERMISSION-HAS-A-SERVICE", known == Set(SetupService.all.flatMap(\.permissions)))
        var state = SetupState()
        state.services = ["calendar"]
        check("Q1-ONLY-WHAT-IS-NEEDED", state.plan.map(\.id) == ["calendar"])
        state.services = ["reminders"]
        check("Q1-REMINDERS-NEEDS-CALENDAR", state.plan.map(\.id) == ["calendar", "reminders"])
        state.services = ["messages", "notes", "mail"]
        check("Q1-SHARED-ASKED-ONCE", state.plan.filter { $0.id == "full_disk_access" }.count == 1 && state.plan.first?.id == "full_disk_access")
        check("Q1-NEXT-SKIPS-ALLOWED", state.next(after: nil, statuses: ["full_disk_access": "granted"])?.id == "accessibility")
        state.skipped = ["accessibility"]
        check("Q1-NEXT-SKIPS-SKIPPED", state.next(after: "full_disk_access", statuses: [:])?.id == "contacts")
        let all = Dictionary(uniqueKeysWithValues: state.plan.map { ($0.id, "authorized") })
        check("Q1-ALL-ALLOWED-ENDS", state.next(after: nil, statuses: all) == nil)
        check("Q1-LIMITED-PHOTOS-IS-ALLOWED", SetupState.allowed("limited") && !SetupState.allowed("denied") && !SetupState.allowed(nil))
        let decoded = try JSONDecoder().decode(SetupState.self, from: JSONEncoder().encode(state))
        check("Q1-RESUME", decoded.services == state.services && decoded.skipped == state.skipped)
        check("Q1-OLD-STATE-STARTS-FRESH", (try? JSONDecoder().decode(SetupState.self, from: Data(#"{"step":2,"skipped":[],"completed":[0,1]}"#.utf8))) == nil)
    }
}
