import Foundation
import AppKit
import Darwin
import MessagesSource
import SourceSupport

public enum HelperCommand {
    @MainActor
    public static func run(_ arguments: [String]) throws {
        if arguments.first == "wire-checks" {
            guard let index = arguments.firstIndex(of: "--fixtures"), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
            let frames = try JSONDecoder().decode([JSON].self, from: Data(contentsOf: URL(fileURLWithPath: arguments[index + 1])))
            for frame in frames {
                let bytes = try PeerWire.encode(frame)
                let decoded = try JSONDecoder().decode(JSON.self, from: bytes.dropFirst(4))
                guard decoded == frame else { throw ProbeFailure.invalidArguments }
                switch frame["t"].string {
                case "Open": guard frame["protocol"].string == "ipc/1", frame["owner_fence"].number > 0 else { throw ProbeFailure.invalidArguments }
                case "Prepare": guard !frame["command_id"].string.isEmpty, parseTimestamp(frame["expires_at"].string) != nil else { throw ProbeFailure.invalidArguments }
                case "Dispatch": guard !frame["command_id"].string.isEmpty, !frame["ticket"].string.isEmpty, frame["owner_fence"].unsigned > 0 else { throw ProbeFailure.invalidArguments }
                case "Outcome": if frame["disposition"].string == "locally_observed" {
                    let evidence = frame["evidence"]
                    guard !evidence["fingerprint"].string.isEmpty, !evidence["capture_checkpoint"].string.isEmpty, !evidence["native_key"].string.isEmpty, evidence["observed"].bool else { throw ProbeFailure.invalidArguments }
                }
                case "Capture": guard !frame["generation"].string.isEmpty, !frame["checkpoint"].string.isEmpty else { throw ProbeFailure.invalidArguments }
                default: break
                }
            }
            print("wire fixtures: \(frames.count) decoded")
            return
        }
        if arguments.first == "session" {
            func option(_ key: String) throws -> String {
                guard let index = arguments.firstIndex(of: key), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
                return arguments[index + 1]
            }
            let socket = try option("--socket")
            let secretFile = try option("--secret-file")
            let home = URL(fileURLWithPath: try option("--home"))
            guard let gatewayPID = Int32(try option("--gateway-pid")), gatewayPID > 1 else { throw ProbeFailure.invalidArguments }
            let lifetime = DispatchSource.makeTimerSource(queue: .global())
            lifetime.schedule(deadline: .now() + 1, repeating: 1)
            lifetime.setEventHandler { if kill(gatewayPID, 0) != 0 { exit(0) } }
            lifetime.resume()
            defer { lifetime.cancel() }
            let peer = try AppleSession(socket: socket, secretFile: secretFile, home: home, gatewayPID: gatewayPID, commandsEnabled: arguments.contains("--lab-commands"))
            var finished = false
            Task { @MainActor in
                let messages = Messages(diagnostics: arguments.contains("--lab-commands") ? home.appendingPathComponent("messages-watch.json") : nil)
                try? await peer.register(messages, readBlob: { handle, offset, length in
                    try await messages.readBlob(handle: handle, offset: offset, length: length)
                })
                try? await peer.run()
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            return
        }
        guard let reportIndex = arguments.firstIndex(of: "--report"), reportIndex + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
        let reportURL = try PrivateReport.validate(arguments[reportIndex + 1])
        if arguments.first == "functional" {
            DispatchQueue.global().asyncAfter(deadline: .now() + 30) {
                try? PrivateReport.write(["state": "timed_out"], to: reportURL)
                exit(1)
            }
            try PrivateReport.write(FunctionalProbe.run(), to: reportURL)
            return
        }
        let session = PermissionSession()
        let deadlineReport = session.report
        let limit: Double = arguments.first == "onboard" ? 300 : 35
        DispatchQueue.global().asyncAfter(deadline: .now() + limit) {
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
