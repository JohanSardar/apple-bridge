import Foundation

@main
struct SetupChecks {
    static func main() throws {
        func check(_ name: String, _ passed: Bool) {
            print("\(name) passed=\(passed)")
            if !passed { exit(1) }
        }
        let standard = GuideStep.steps([])
        check("Q1-CATALOG", standard.map(\.id) == ["contacts", "calendar", "reminders", "photos", "full_disk_access", "screen_recording"] && GuideStep.catalog.allSatisfy { !$0.title.isEmpty && !$0.reason.isEmpty && ($0.kind == .prompt) == ($0.settingsURL == nil) })
        check("Q1-ORDER", GuideStep.steps(["screen_recording", "contacts", "full_disk_access"]).map(\.id) == ["contacts", "screen_recording", "full_disk_access"])
        var flow = GuideFlow(steps: standard)
        check("Q1-FRESH", flow.index == 0 && flow.moment == .asking && flow.position == "STEP 1 OF 6")
        flow.dropAlreadyAllowed(["calendar"])
        check("Q1-ALREADY-ALLOWED", flow.steps.count == 5 && flow.allowed == ["calendar"] && flow.current?.id == "contacts")
        flow.allow()
        flow.next()
        flow.decline()
        flow.next()
        flow.skip()
        check("Q1-DENIED-CONTINUE", flow.current?.id == "full_disk_access" && flow.moment == .waitingForDrop && flow.declined == ["reminders"] && flow.skipped == ["photos"])
        flow.dropped()
        check("Q1-CONFIRMING", flow.moment == .confirming)
        flow.passwordClosed(granted: false)
        check("Q1-PASSWORD-CANCEL", flow.moment == .cancelledPassword && flow.current?.id == "full_disk_access" && !flow.finished)
        flow.dropped()
        flow.passwordClosed(granted: true)
        check("Q1-PASSWORD-OK", flow.moment == .allowed && flow.allowed.contains("full_disk_access"))
        flow.next()
        let saved = try JSONEncoder().encode(flow)
        check("Q1-MODEL-RESUME", GuideFlow.resumable(saved) == flow && GuideFlow.resumable(saved, now: Date().addingTimeInterval(GuideFlow.resumeWindow + 1)) == nil)
        var reopened = try JSONDecoder().decode(GuideFlow.self, from: saved)
        reopened.index = 4
        reopened.moment = .allowed
        reopened.dropAlreadyAllowed(["contacts", "calendar", "full_disk_access"])
        check("Q1-RESUME-AFTER-QUIT", reopened.current?.id == "screen_recording" && reopened.moment == .waitingForDrop && reopened.position == "STEP 5 OF 5")
        flow.allow()
        flow.next()
        check("Q1-COMPLETE", flow.finished && flow.moment == .finished && flow.summary["outcome"] == "finished" && flow.summary["allowed"] == "calendar,contacts,full_disk_access,screen_recording")
        check("Q1-FINISHED-NOT-RESUMED", GuideFlow.resumable(try JSONEncoder().encode(flow)) == nil)
        var empty = GuideFlow(steps: GuideStep.steps(["unknown"]))
        empty.skip()
        check("Q1-ALL-SKIPPED", empty.finished && GuideFlow(steps: []).moment == .finished)
    }
}
