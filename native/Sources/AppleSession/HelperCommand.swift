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
        if arguments.first == "permission-status-proof" || arguments.first == "permission-status" || arguments.first == "permission-request" || arguments.first == "permission-fake" {
            guard let index = arguments.firstIndex(of: "--report"), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
            let report = try PrivateReport.validate(arguments[index + 1])
            var finished = false
            var failure: (any Error)?
            Task { @MainActor in
                do {
                    if arguments.first == "permission-status-proof" {
                        try PrivateReport.write(try await NewPermissions.statusProof(), to: report)
                    } else if arguments.first == "permission-status" {
                        try PrivateReport.write(await NewPermissions.status(), to: report)
                    } else if arguments.first == "permission-fake" {
                        var counts: [String: String] = [:]
                        for name in NewPermissions.names {
                            var calls = Dictionary(uniqueKeysWithValues: NewPermissions.names.map { ($0, 0) })
                            let adapters = Dictionary(uniqueKeysWithValues: NewPermissions.names.map { selected in
                                (selected, { @MainActor in calls[selected, default: 0] += 1 } as NewPermissions.RequestAdapter)
                            })
                            let before = NewPermissions.requestAPICalls
                            try await NewPermissions.requestNative(name, adapters: adapters)
                            guard NewPermissions.requestAPICalls == before + 1, calls[name] == 1, calls.values.reduce(0, +) == 1 else { throw ProbeFailure.invalidArguments }
                            counts[name] = String(calls[name] ?? 0)
                        }
                        var unknownCalls = 0
                        let adapters = Dictionary(uniqueKeysWithValues: NewPermissions.names.map { name in
                            (name, { @MainActor in unknownCalls += 1 } as NewPermissions.RequestAdapter)
                        })
                        let before = NewPermissions.requestAPICalls
                        do {
                            try await NewPermissions.requestNative("unknown_permission", adapters: adapters)
                            throw ProbeFailure.invalidArguments
                        } catch HelperCommandFailure.unknownCommand {
                            guard unknownCalls == 0, NewPermissions.requestAPICalls == before else { throw ProbeFailure.invalidArguments }
                            counts["unknown_rejected"] = "1"
                        }
                        try PrivateReport.write(counts, to: report)
                    } else {
                        guard arguments.count > 1 else { throw ProbeFailure.invalidArguments }
                        try await NewPermissions.requestNative(arguments[1])
                        try await Task.sleep(for: .seconds(2))
                        try PrivateReport.write(["request": arguments[1]], to: report)
                    }
                } catch { failure = error }
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            if let failure { throw failure }
            return
        }
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
            let lifetime = DispatchSource.makeProcessSource(identifier: gatewayPID, eventMask: .exit, queue: .global())
            lifetime.setEventHandler { @Sendable in exit(0) }
            lifetime.resume()
            defer { lifetime.cancel() }
            if kill(gatewayPID, 0) != 0 { return }
            let peer = try AppleSession(socket: socket, secretFile: secretFile, home: home, gatewayPID: gatewayPID, commandsEnabled: arguments.contains("--lab-commands"))
            Task { @MainActor in
                if !arguments.contains("--lab-commands") { _ = try? await MessagesSelfTest.shared.run() }
                await SessionSources.register(into: peer, home: home, labEnabled: arguments.contains("--lab-commands"))
                try? await peer.run()
                NSApplication.shared.terminate(nil)
            }
            NSApplication.shared.run()
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
