import Foundation
import AppKit
import Darwin
import MessagesSource

public enum HelperCommand {
    @MainActor
    public static func run(_ arguments: [String]) throws {
        guard let reportIndex = arguments.firstIndex(of: "--report"), reportIndex + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
        let reportURL = try PrivateReport.validate(arguments[reportIndex + 1])
        let session = PermissionSession()
        let deadlineReport = session.report
        DispatchQueue.global().asyncAfter(deadline: .now() + 35) {
            try? PrivateReport.write(deadlineReport.snapshot(), to: reportURL)
            exit(1)
        }
        session.refresh()
        if arguments.first == "status" {
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            return
        }
        if arguments.first == "onboard" {
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            let report = session.report
            DispatchQueue.global().asyncAfter(deadline: .now() + 30) {
                try? PrivateReport.write(report.snapshot(), to: reportURL)
                exit(0)
            }
            session.onboard()
            let deadline = Date().addingTimeInterval(20)
            while Date() < deadline {
                RunLoop.current.run(until: Date().addingTimeInterval(0.25))
            }
            session.refresh()
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            return
        }
        guard arguments.first == "probe", arguments.count > 1 else { throw ProbeFailure.invalidArguments }
        guard reportURL.lastPathComponent != "self-handles.json" || arguments[1] == "self-handles" else { throw ProbeFailure.invalidArguments }
        try PrivateReport.write(["state": "running"], to: reportURL)
        DispatchQueue.global().asyncAfter(deadline: .now() + 35) {
            try? PrivateReport.write(["state": "timed_out", "disposition": "indeterminate"], to: reportURL)
            exit(1)
        }
        do {
            let result = try MessagesProbes.run(arguments)
            try PrivateReport.write(result, to: reportURL)
        } catch {
            try PrivateReport.write(["state": "unavailable", "reason": String(describing: error)], to: reportURL)
            throw error
        }
    }
}
