#if LAB
import Foundation
import AppKit
import Darwin
import MessagesSource
import RemindersStore
import SourceSupport
import NotesSource

enum HelperLabCommand {
    @MainActor
    static func run(_ arguments: [String]) throws -> Bool {
        switch arguments.first {
        case "photos-real-checks":
            let reportURL = try report(arguments)
            guard arguments.count > 1 else { throw ProbeFailure.invalidArguments }
            var finished = false
            var failure: (any Error)?
            Task { @MainActor in
                do {
                    let result = try await PhotosRealProof.run(arguments[1])
                    try JSONEncoder().encode(result).write(to: reportURL, options: .atomic)
                } catch { failure = error }
                finished = true
            }
            while !finished { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            if let failure { throw failure }
            return true
        case "media-cleanup-checks":
            guard let index = arguments.firstIndex(of: "--fixtures"), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
            let fixture = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: URL(fileURLWithPath: arguments[index + 1])))
            let proof = try MessagesMediaCleanupProof(registration: fixture["registration"], tag: fixture["tag"].string)
            let row = fixture["row"]
            guard let rowID = Int64(exactly: row["row_id"].number) else { throw ProbeFailure.invalidArguments }
            let authorized = proof.authorizes(rowID: rowID, attachments: row["attachments"].array.map(\.string), peers: row["peers"].array.map(\.string), own: Set(fixture["own"].array.map { MessagesProbes.normalize($0.string) }))
            let names = Set(proof.targetNames(row["attachments"].array.map(\.string)))
            let candidates = fixture["candidates"].array.map { candidate -> JSON in
                guard !candidate["description"].string.isEmpty else { return candidate }
                var value = candidate.object
                value["outgoing"] = MessagesMediaCleanupProof.direction(candidate["description"].string, names: names).map(JSON.bool) ?? .null
                value["values"] = .array(candidate["values"].array + [candidate["description"]])
                return .object(value)
            }
            let target = MessagesMediaCleanupProof.targetIndex(candidates, names: names, outgoing: row["is_from_me"].bool)
            let result = JSON.object(["authorized": .bool(authorized), "target_verified": .bool(authorized && target != nil)])
            print(String(decoding: try JSONEncoder().encode(result), as: UTF8.self))
            return true
        case "wire-checks":
            guard let index = arguments.firstIndex(of: "--fixtures"), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
            let frames = try JSONDecoder().decode([JSON].self, from: Data(contentsOf: URL(fileURLWithPath: arguments[index + 1])))
            for frame in frames {
                let bytes = try PeerWire.encode(frame)
                let decoded = try JSONDecoder().decode(JSON.self, from: bytes.dropFirst(4))
                guard decoded == frame else { throw ProbeFailure.invalidArguments }
                switch frame["t"].string {
                case "Open": guard frame["protocol"].string == "ipc/1", frame["owner_fence"].number > 0 else { throw ProbeFailure.invalidArguments }
                case "Prepare":
                    _ = try CommandOrigin(frame)
                    guard !frame["command_id"].string.isEmpty, parseTimestamp(frame["expires_at"].string) != nil else { throw ProbeFailure.invalidArguments }
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
            return true
        case "reminders-store-profile":
            let reportURL = try report(arguments)
            var result: Result<JSON, any Error>?
            var stage = "cold_responsiveness"
            Task { @MainActor in
                do {
                    let responsiveness = try await RemindersStoreResponsiveness.run()
                    stage = "warm_profile"
                    var profile = try await RemindersStoreProfile.typed().object
                    profile["responsiveness"] = responsiveness
                    result = .success(.object(profile))
                }
                catch { result = .failure(error) }
            }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.005)) }
            let profile: JSON
            do { profile = try result!.get() }
            catch {
                let failure = error as NSError
                let reason = error is SourceFailure ? String(describing: error) : failure.domain + ":" + String(failure.code)
                try PrivateReport.write(["status": "unavailable", "stage": stage, "reason": reason], to: reportURL)
                throw error
            }
            try JSONEncoder().encode(profile).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return true
        case "reminders-store-flag-proof":
            let reportURL = try report(arguments)
            func option(_ key: String) throws -> String {
                guard let index = arguments.firstIndex(of: key), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
                return arguments[index + 1]
            }
            let tag = try option("--tag"), item = try option("--item-id"), list = try option("--list-id")
            var result: Result<JSON, any Error>?
            Task { @MainActor in
                do { result = .success(try await RemindersFlagProof.run(tag: tag, itemIdentifier: item, listIdentifier: list)) }
                catch { result = .failure(error) }
            }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.005)) }
            let proof = try result!.get()
            try JSONEncoder().encode(proof).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return true
        case "reminders-store-blob-fixture":
            let reportURL = try report(arguments)
            var result: Result<JSON, any Error>?
            Task { @MainActor in
                do {
                    let proof: JSON
                    if arguments.contains("--cleanup") { proof = try await RemindersBlobFixture.recover(journal: reportURL) }
                    else { proof = try await RemindersBlobFixture.run(journal: reportURL) }
                    result = .success(proof)
                }
                catch { result = .failure(error) }
            }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.005)) }
            try JSONEncoder().encode(try result!.get()).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return true
        case "reminders-store-proof":
            let reportURL = try report(arguments)
            func option(_ key: String) throws -> String {
                guard let index = arguments.firstIndex(of: key), index + 1 < arguments.count else { throw ProbeFailure.invalidArguments }
                return arguments[index + 1]
            }
            let proof = try RemindersStoreProof.run(tag: option("--tag"), itemIdentifier: option("--item-id"), listIdentifier: option("--list-id"))
            try JSONEncoder().encode(proof).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return true
        case "calendar-parity-probe":
            let reportURL = try report(arguments)
            var result: Result<JSON, any Error>?
            Task { @MainActor in
                do { result = .success(try await CalendarParityProbe.run(requestConsent: arguments.contains("--request-consent"))) }
                catch { result = .failure(error) }
            }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.005)) }
            try JSONEncoder().encode(try result!.get()).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
            return true
        case "notes-parity-probe":
            let reportURL = try report(arguments)
            var result: JSON?
            Task { @MainActor in result = await NotesParityProbe.run() }
            while result == nil { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            try JSONEncoder().encode(result!).write(to: reportURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: reportURL.path)
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
                    if arguments.count > 1 && arguments[1] == "cleanup-tagged-self" { data = try await MessagesCleanup.run(arguments) }
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
