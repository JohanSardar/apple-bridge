import Foundation
import AppKit
import Darwin
import MessagesSource
import NotesSource
import ContactsSource
import SourceSupport

public enum HelperCommand {
    @MainActor
    public static func run(_ arguments: [String]) throws {
        #if LAB
        if try HelperLabCommand.run(arguments) { return }
        #endif
        if arguments.first == "session" {
            func option(_ key: String) throws -> String {
                guard let index = arguments.firstIndex(of: key), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
                return arguments[index + 1]
            }
            let socket = try option("--socket")
            let secretFile = try option("--secret-file")
            let home = URL(fileURLWithPath: try option("--home"))
            guard let gatewayPID = Int32(try option("--gateway-pid")), gatewayPID > 1 else { throw ProbeFailure.invalidArguments }
            let activity = ProcessInfo.processInfo.beginActivity(options: .idleSystemSleepDisabled, reason: "Apple Bridge gateway session")
            defer { ProcessInfo.processInfo.endActivity(activity) }
            let timers = CaptureTrace(url: arguments.contains("--lab-commands") ? home.appendingPathComponent("session-timers.jsonl") : nil)
            let lifetime = DispatchSource.makeTimerSource(queue: .global())
            lifetime.schedule(deadline: .now() + 1, repeating: 1)
            lifetime.setEventHandler { @Sendable in timers.record("lifetime_timer"); if kill(gatewayPID, 0) != 0 { exit(0) } }
            lifetime.resume()
            defer { lifetime.cancel() }
            let peer = try AppleSession(socket: socket, secretFile: secretFile, home: home, gatewayPID: gatewayPID, commandsEnabled: arguments.contains("--lab-commands"))
            var finished = false
            Task { @MainActor in
                if !arguments.contains("--lab-commands") { _ = try? await MessagesSelfTest.shared.run() }
                await SessionSources.register(into: peer, home: home, labEnabled: arguments.contains("--lab-commands"))
                try? await peer.run()
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            return
        }
        guard let command = arguments.first, ["selftest", "status", "onboard", "authorize-reminders"].contains(command) else { throw HelperCommandFailure.unknownCommand }
        guard let reportIndex = arguments.firstIndex(of: "--report"), reportIndex + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
        let reportURL = try PrivateReport.validate(arguments[reportIndex + 1])
        if arguments.first == "selftest", arguments.dropFirst().first == "contacts-groups" {
            let descriptor = Darwin.open(PrivateReport.directory.appendingPathComponent("contacts-groups-selftest.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
            guard descriptor >= 0 else { throw ProbeFailure.unavailable }
            defer { Darwin.close(descriptor) }
            guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw ProbeFailure.unavailable }
            var finished = false
            Task { @MainActor in
                do {
                    let source = try ContactsSource(home: PrivateReport.directory, commandsEnabled: true)
                    let result = await source.proveGroups()
                    try JSONEncoder().encode(result).write(to: reportURL, options: .atomic)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
                } catch { try? PrivateReport.write(["state": "limited", "reason": String(describing: error)], to: reportURL) }
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            return
        }
        if arguments.first == "selftest",arguments.dropFirst().first == "messages-variants" {
            DispatchQueue.global().asyncAfter(deadline:.now()+600) { @Sendable in
                try? PrivateReport.write(["state":"limited","reason":"variant_proof_deadline","disposition":"indeterminate"],to:reportURL)
                exit(1)
            }
            var finished=false
            Task { @MainActor in
                do { let result=try await MessagesSendVariantProof.run();try JSONEncoder().encode(result).write(to:reportURL,options:.atomic) }
                catch { try? PrivateReport.write(["state":"limited","reason":String(describing:error)],to:reportURL) }
                finished=true
            }
            while !finished { RunLoop.current.run(until:Date().addingTimeInterval(0.05)) }
            return
        }
        if arguments.first == "selftest", ["messages", "messages-parity"].contains(arguments.dropFirst().first ?? "") {
            var finished = false
            DispatchQueue.global().asyncAfter(deadline: .now() + 600) { @Sendable in
                try? PrivateReport.write(["state": "interrupted", "reason": "selftest_deadline", "disposition": "indeterminate"], to: reportURL)
                exit(1)
            }
            Task { @MainActor in
                do {
                    let result: JSON
                    if arguments.dropFirst().first == "messages-parity" { result = try await MessagesParitySelfTest.shared.run(force: arguments.contains("--force"), includeUI: !arguments.contains("--no-ui")) }
                    else { result = try await MessagesSelfTest.shared.run(force: arguments.contains("--force"), includeUI: !arguments.contains("--no-ui")) }
                    try JSONEncoder().encode(result).write(to: reportURL, options: .atomic)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
                } catch { try? PrivateReport.write(["state": "unavailable", "reason": String(describing: error)], to: reportURL) }
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            return
        }
        let session = PermissionSession()
        let deadlineReport = session.report
        let limit: Double = arguments.first == "onboard" ? 300 : 35
        DispatchQueue.global().asyncAfter(deadline: .now() + limit) { @Sendable in
            try? PrivateReport.write(deadlineReport.snapshot(), to: reportURL)
            exit(1)
        }
        session.refresh()
        if arguments.first == "status" {
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            return
        }
        if arguments.first == "authorize-reminders" {
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            var finished = false
            Task { @MainActor in
                await session.authorizeReminders()
                try PrivateReport.write(session.report.snapshot(), to: reportURL)
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            return
        }
        if arguments.first == "onboard" {
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            var finished = false
            Task { @MainActor in
                await session.onboard()
                finished = true
            }
            while !finished {
                RunLoop.current.run(until: Date().addingTimeInterval(0.25))
            }
            let settle = Date().addingTimeInterval(5)
            while Date() < settle {
                RunLoop.current.run(until: Date().addingTimeInterval(0.25))
            }
            session.refresh()
            try PrivateReport.write(session.report.snapshot(), to: reportURL)
            return
        }
        throw ProbeFailure.invalidArguments
    }
}

enum HelperCommandFailure: Error {
    case unknownCommand
}
