#if LAB
import Foundation
import AppKit
import Darwin
import MessagesSource
import SourceSupport
import NotesSource
import FocusSource

enum HelperLabCommand {
    @MainActor
    static func run(_ arguments: [String]) throws -> Bool {
        switch arguments.first {
        case "focus-phone-pack":
            let reportURL = try report(arguments)
            guard arguments.count > 1 else { throw ProbeFailure.invalidArguments }
            var result: Result<JSON, any Error>?
            Task { @MainActor in
                do { result = .success(try await FocusPhonePack.install(URL(fileURLWithPath: arguments[1]))) }
                catch { result = .failure(error) }
            }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            try JSONEncoder().encode(result!.get()).write(to: reportURL, options: .atomic)
            return true
        case "focus-real-checks":
            let reportURL = try report(arguments)
            guard arguments.count > 1 else { throw ProbeFailure.invalidArguments }
            var finished = false
            var failure: (any Error)?
            Task { @MainActor in
                do {
                    let result = try await FocusQualification.run(arguments[1])
                    try JSONEncoder().encode(result).write(to: reportURL, options: .atomic)
                } catch { failure = error }
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            if let failure { throw failure }
            return true
        case "mac-notify-proof":
            let reportURL = try report(arguments)
            var result: JSON?
            Task { @MainActor in result = await MacNotifyProof.run() }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            try JSONEncoder().encode(result!).write(to: reportURL, options: .atomic)
            return true
        case "calendar-facts-proof":
            let reportURL = try report(arguments)
            var result: (state: String, reason: String)?
            Task { @MainActor in result = await ExternalChange.calendarEnumerationProof() }
            let deadline = Date().addingTimeInterval(20)
            while result == nil, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            try PrivateReport.write(["state": result?.state ?? "blocked", "reason": result?.reason ?? "enumeration_waited_for_reminders"], to: reportURL)
            return true
        case "external-change":
            let reportURL = try report(arguments)
            guard let index = arguments.firstIndex(of: "--request"), index + 1 < arguments.count,
                  let request = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: arguments[index + 1]))) as? [String: Any] else { throw ProbeFailure.invalidArguments }
            try PrivateReport.write(ExternalChange.run(request), to: reportURL)
            return true
        case "notes-probe":
            let reportURL = try report(arguments)
            var error: NSDictionary?
            let result = NSAppleScript(source: "with timeout of 5 seconds\ntell application \"Notes\" to count accounts\nend timeout")?.executeAndReturnError(&error)
            try PrivateReport.write(["state": error == nil ? "ready" : "unavailable", "error_code": error?["NSAppleScriptErrorNumber"] as? Int ?? 0, "count": result?.int32Value ?? 0], to: reportURL)
            return true
        case "functional":
            let reportURL = try report(arguments)
            DispatchQueue.global().asyncAfter(deadline: .now() + 30) { @Sendable in
                try? PrivateReport.write(["state": "timed_out"], to: reportURL)
                exit(1)
            }
            try PrivateReport.write(FunctionalProbe.run(), to: reportURL)
            return true
        case "probe":
            let reportURL = try report(arguments)
            try PrivateReport.write(["state":"running"], to:reportURL)
            DispatchQueue.global().asyncAfter(deadline:.now()+(arguments.contains("cleanup-tagged-self") ? 600 : 35)) { @Sendable in
                try? PrivateReport.write(["state":"timed_out","disposition":"indeterminate"],to:reportURL)
                exit(1)
            }
            var finished=false
            Task { @MainActor in
                do {
                    let data: Data
                    if arguments.count > 1 && arguments[1] == "cleanup-tagged-self" {
                        guard let index = arguments.firstIndex(of: "--tag"), index + 1 < arguments.count, arguments.contains("--owned-only") else { throw ProbeFailure.invalidArguments }
                        try await RustSelfTest.shared.cleanup(tag: arguments[index + 1], report: reportURL.path)
                        finished = true
                        return
                    }
                    else { data=try JSONSerialization.data(withJSONObject:MessagesProbes.run(arguments)) }
                    try data.write(to:reportURL,options:.atomic)
                    try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:reportURL.path)
                } catch {try? PrivateReport.write(["state":"unavailable","reason":String(describing:error)],to:reportURL)}
                finished=true
            }
            while !finished {RunLoop.current.run(until:Date().addingTimeInterval(0.05))}
            return true
        default: return false
        }
    }

    private static func report(_ arguments: [String]) throws -> URL {
        guard let index = arguments.firstIndex(of: "--report"), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
        return try PrivateReport.validate(arguments[index + 1])
    }
}
#endif
