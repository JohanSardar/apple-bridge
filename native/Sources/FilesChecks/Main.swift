import Foundation
import Darwin
import CryptoKit
import FilesSource
import SourceSupport

@main
struct Checks {
    static func commands() async throws {
        let fm = FileManager.default
        let simulated = CommandLine.arguments.contains("FILE-COMMANDS")
        let home = simulated ? fm.temporaryDirectory.appendingPathComponent(UUID().uuidString) : fm.homeDirectoryForCurrentUser
        if simulated { try fm.createDirectory(at: home, withIntermediateDirectories: true) }
        defer { if simulated { try? fm.removeItem(at: home) } }
        let source = try FilesSource(home: home)
        let root = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
        let folder = root.appendingPathComponent("[ab-test " + UUID().uuidString + "]")
        try fm.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: folder) }
        let parent = folder.lastPathComponent
        func run(_ op: String, _ extra: [String: JSON]) async throws -> JSON {
            var args = extra
            args["root"] = .string(root.path)
            let prepared = try await source.prepare(.object(["action": .object(["op": .string(op), "args": .object(args)])]))
            let evidence = try await source.dispatch(prepared)
            guard evidence["observed"].bool else { throw SourceFailure.corrupt }
            return evidence
        }
        for (op, text) in [("files.create", "first"), ("files.write", "second")] {
            let data = Data(text.utf8)
            let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            let evidence = try await run(op, ["parent": .string(parent), "name": .string("a.txt"), "content_base64": .string(data.base64EncodedString()), "digest": .string(digest)])
            guard evidence["digest"].string == digest, try Data(contentsOf: folder.appendingPathComponent("a.txt")) == data else { throw SourceFailure.corrupt }
        }
        let link = folder.appendingPathComponent("link")
        try fm.createSymbolicLink(at: link, withDestinationURL: fm.temporaryDirectory)
        for path in ["../escape", parent + "/link/escape"] {
            do {
                _ = try await run("files.trash", ["path": .string(path)])
                throw SourceFailure.corrupt
            } catch SourceFailure.invalid {}
        }
        _ = try await run("files.rename", ["path": .string(parent + "/a.txt"), "parent": .string(parent), "name": .string("b.txt")])
        let target = folder.appendingPathComponent("target")
        try fm.createDirectory(at: target, withIntermediateDirectories: true)
        _ = try await run("files.move", ["path": .string(parent + "/b.txt"), "parent": .string(parent + "/target"), "name": .string("c.txt")])
        let file = target.appendingPathComponent("c.txt")
        guard fm.fileExists(atPath: file.path), !fm.fileExists(atPath: folder.appendingPathComponent("b.txt").path) else { throw SourceFailure.corrupt }
        _ = try await run("files.set_tags", ["path": .string(parent + "/target/c.txt"), "tags": .array([.string("Bridge Test")])])
        guard try file.resourceValues(forKeys: [.tagNamesKey]).tagNames == ["Bridge Test"] else { throw SourceFailure.corrupt }
        if simulated {
            for op in ["files.download", "files.evict"] {
                do {
                    _ = try await run(op, ["path": .string(parent + "/target/c.txt")])
                    throw SourceFailure.corrupt
                } catch SourceFailure.unsupported {}
            }
        }
        let trash = try await run("files.trash", ["path": .string(parent + "/target/c.txt")])
        let trashURL = URL(fileURLWithPath: trash["trash_path"].string)
        guard try Data(contentsOf: trashURL) == Data("second".utf8) else { throw SourceFailure.corrupt }
        try fm.moveItem(at: trashURL, to: file)
        guard fm.fileExists(atPath: file.path) else { throw SourceFailure.corrupt }
    }
    static func main() async throws {
        if CommandLine.arguments.contains("FILE-COMMANDS") || CommandLine.arguments.contains("FILE-CRUD-REAL") { try await commands(); return }
        if CommandLine.arguments.contains("FILE-REPLICA") { throw SourceFailure.capabilityUnproven }
        let home = FileManager.default.homeDirectoryForCurrentUser
        let root = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
        let source = try FilesSource()
        let folder = root.appendingPathComponent("[ab-test " + UUID().uuidString + "]")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let file = folder.appendingPathComponent("synthetic.txt")
        let token = "abtest" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        try Data(token.utf8).write(to: file)
        let (rows, complete) = try await source.snapshot(root)
        guard complete, rows.contains(where: { $0["native_key"].string == file.path }) else { throw SourceFailure.corrupt }
        let args: JSON = .object(["root": .string(root.path), "path": .string(folder.lastPathComponent + "/synthetic.txt")])
        let result = try await source.read(args)
        guard result["size"].number == Double(token.utf8.count), result["digest"].string.count == 64 else { throw SourceFailure.corrupt }
        do {
            _ = try await source.read(.object(["root": .string(root.path), "path": .string("../outside")]))
            throw SourceFailure.corrupt
        } catch SourceFailure.invalid {}
        let scenario = CommandLine.arguments.last ?? ""
        if scenario == "FILE-SEARCH-REAL" {
            let deadline = Date().addingTimeInterval(30)
            var found = false
            repeat {
                let results = try await source.search(.object(["query": .string(token), "field": .string("content")]))
                found = results["items"].array.contains { $0["path"].string == file.path }
                if found { break }
                try await Task.sleep(for: .seconds(1))
            } while Date() < deadline
            guard found else { throw SourceFailure.capabilityUnproven }
        } else if scenario == "FILE-OFFLOAD-REAL" {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/brctl")
            process.arguments = ["evict", file.path]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { throw SourceFailure.capabilityUnproven }
            let evictionDeadline = Date().addingTimeInterval(15)
            while try file.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]).ubiquitousItemDownloadingStatus != .notDownloaded {
                guard Date() < evictionDeadline else { throw SourceFailure.capabilityUnproven }
                try await Task.sleep(for: .milliseconds(100))
            }
            let reread = try await source.read(args)
            guard reread == result else { throw SourceFailure.corrupt }
        } else if scenario == "FILE-IDLE" {
            let guardValue = FileScanGuard()
            var scanGuard = guardValue
            _ = scanGuard.finish(root: root.path, keys: ["one"], complete: true)
            guard scanGuard.finish(root: root.path, keys: [], complete: false).isEmpty,
                  scanGuard.finish(root: root.path, keys: [], complete: true) == ["one"] else { throw SourceFailure.corrupt }
        }
        if CommandLine.arguments.contains("--real-idle") {
            let capture = Task { try await source.capture(.null, into: IdleSink()) }
            defer { capture.cancel() }
            let start = clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID)
            let wall = Date()
            try await Task.sleep(for: .seconds(300))
            let cpu = Double(clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID) - start) / 1e9
            let percent = cpu / Date().timeIntervalSince(wall) * 100
            print("idle_cpu_percent=\(percent)")
            guard percent < 2 else { throw SourceFailure.capabilityUnproven }
        }
        try FileManager.default.removeItem(at: file)
        let (after, _) = try await source.snapshot(root)
        guard !after.contains(where: { $0["native_key"].string == file.path }) else { throw SourceFailure.corrupt }

    }
}

struct IdleSink: ObservationSink {
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
}
