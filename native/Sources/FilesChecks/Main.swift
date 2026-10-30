import Foundation
import Darwin
import CryptoKit
import CoreGraphics
import ImageIO
import PDFKit
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
        defer { try? cleanupCommands(folder) }
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
        let sink = CommandSink()
        try await source.reconcile(into: sink)
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
        try await source.reconcile(into: sink)
        let captured = await sink.rows
        guard captured.contains(where: { $0["kind"].string == "absent" && $0["native_key"].string == folder.appendingPathComponent("a.txt").path }), captured.contains(where: { $0["kind"].string == "upsert" && $0["native_key"].string == target.appendingPathComponent("c.txt").path }) else { throw SourceFailure.corrupt }
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
        try cleanupCommands(folder)
    }
    static func cleanupCommands(_ folder: URL) throws {
        let fm = FileManager.default
        guard fm.fileExists(atPath: folder.path) else { return }
        guard folder.lastPathComponent.hasPrefix("[ab-test "), folder.deletingLastPathComponent().lastPathComponent == "Bridge Test" else { throw SourceFailure.corrupt }
        var failed = false
        guard let tree = fm.enumerator(at: folder, includingPropertiesForKeys: [.isSymbolicLinkKey, .isDirectoryKey, .fileSizeKey], options: [], errorHandler: { _, _ in failed = true; return false }) else { throw SourceFailure.corrupt }
        while let url = tree.nextObject() as? URL {
            guard url.path.hasPrefix(folder.path + "/") else { throw SourceFailure.corrupt }
            let path = String(url.path.dropFirst(folder.path.count + 1))
            let values = try url.resourceValues(forKeys: [.isSymbolicLinkKey, .isDirectoryKey, .fileSizeKey])
            if values.isSymbolicLink == true {
                tree.skipDescendants()
                let destination = try fm.destinationOfSymbolicLink(atPath: url.path)
                guard path == "link", URL(fileURLWithPath: destination).resolvingSymlinksInPath().path == fm.temporaryDirectory.resolvingSymlinksInPath().path else { throw SourceFailure.corrupt }
            } else if values.isDirectory == true {
                guard path == "target" else { throw SourceFailure.corrupt }
            } else {
                guard ["a.txt", "b.txt", "target/c.txt"].contains(path), let size = values.fileSize, size <= 6 else { throw SourceFailure.corrupt }
                let data = try Data(contentsOf: url)
                guard data == Data("first".utf8) || data == Data("second".utf8) else { throw SourceFailure.corrupt }
            }
        }
        guard !failed else { throw SourceFailure.corrupt }
        try fm.removeItem(at: folder)
    }
    static func transforms() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: home) }
        let source = try FilesSource(home: home)
        let root = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
        guard let context = CGContext(data: nil, width: 16, height: 12, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue), let image = context.makeImage() else { throw SourceFailure.corrupt }
        let png = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(png, "public.png" as CFString, 1, nil) else { throw SourceFailure.corrupt }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { throw SourceFailure.corrupt }
        func stage(_ data: Data, _ name: String) async throws -> String {
            try data.write(to: root.appendingPathComponent(name))
            return try await source.read(.object(["root": .string(root.path), "path": .string(name)]))["digest"].string
        }
        func run(_ args: JSON) async throws -> [JSON] {
            let prepared = try await source.prepare(.object(["action": .object(["op": .string("files.transform"), "args": args])]))
            return try await source.dispatch(prepared)["outputs"].array
        }
        func bytes(_ blob: JSON) async throws -> Data {
            let chunk = try await source.readBlob(blob["handle"].string, offset: 0, length: 131072)
            guard let data = Data(base64Encoded: chunk["bytes"].string), data.count == Int(blob["size"].number) else { throw SourceFailure.corrupt }
            return data
        }
        let input = try await stage(png as Data, "image.png")
        for operation in ["convert", "resize"] {
            let outputs = try await run(.object(["operation": .string(operation), "inputs": .array([.string(input)]), "format": .string("jpeg"), "width": .number(8), "height": .number(6)]))
            guard outputs.count == 1, outputs[0]["media_type"].string == "image/jpeg", let decoded = CGImageSourceCreateWithData(try await bytes(outputs[0]) as CFData, nil), CGImageSourceGetType(decoded) as String? == "public.jpeg", let result = CGImageSourceCreateImageAtIndex(decoded, 0, nil), result.width == (operation == "resize" ? 8 : 16), result.height == (operation == "resize" ? 6 : 12) else { throw SourceFailure.corrupt }
        }
        let pdf = PDFDocument()
        guard let page = PDFPage(image: NSImage(cgImage: image, size: .zero)) else { throw SourceFailure.corrupt }
        pdf.insert(page, at: 0)
        let digest = try await stage(pdf.dataRepresentation()!, "one.pdf")
        let combined = try await run(.object(["operation": .string("combine"), "inputs": .array([.string(digest), .string(digest)])]))
        guard combined.count == 1, PDFDocument(data: try await bytes(combined[0]))?.pageCount == 2 else { throw SourceFailure.corrupt }
        let split = try await run(.object(["operation": .string("split"), "inputs": .array([combined[0]["digest"]])]))
        guard split.count == 2 else { throw SourceFailure.corrupt }
        for blob in split {
            guard blob["media_type"].string == "application/pdf", PDFDocument(data: try await bytes(blob))?.pageCount == 1 else { throw SourceFailure.corrupt }
        }
        do {
            _ = try await run(.object(["operation": .string("combine"), "inputs": .array([.string(input)])]))
            throw SourceFailure.corrupt
        } catch SourceFailure.invalid {}
        print("transforms=4 passed=true")
    }
    static func main() async throws {
        if CommandLine.arguments.contains("FILE-COMMANDS") || CommandLine.arguments.contains("FILE-CRUD-REAL") { try await commands(); return }
        if CommandLine.arguments.last == "FILES-TRANSFORM" {
            try await transforms()
            return
        }
        let home = FileManager.default.homeDirectoryForCurrentUser
        let root = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
        let source = try FilesSource()
        let folder = root.appendingPathComponent("[ab-test " + UUID().uuidString + "]")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let file = folder.appendingPathComponent("[ab-test] synthetic.txt")
        let token = "abtest" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        try Data(token.utf8).write(to: file)
        let (rows, complete) = try await source.snapshot(root)
        guard complete, rows.contains(where: { $0["native_key"].string == file.path }) else { throw SourceFailure.corrupt }
        let args: JSON = .object(["root": .string(root.path), "path": .string(folder.lastPathComponent + "/" + file.lastPathComponent)])
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
            func cloudValues() throws -> URLResourceValues {
                var refreshed = URL(fileURLWithPath: file.path)
                refreshed.removeAllCachedResourceValues()
                return try refreshed.resourceValues(forKeys: [.isUbiquitousItemKey, .ubiquitousItemIsUploadedKey, .ubiquitousItemDownloadingStatusKey])
            }
            let backingDeadline = Date().addingTimeInterval(30)
            while try cloudValues().isUbiquitousItem != true {
                guard Date() < backingDeadline else {
                    print("offload_fixture_icloud_backed=false")
                    throw SourceFailure.capabilityUnproven
                }
                try await Task.sleep(for: .milliseconds(500))
            }
            let uploadDeadline = Date().addingTimeInterval(60)
            while try cloudValues().ubiquitousItemIsUploaded != true {
                guard Date() < uploadDeadline else {
                    print("offload_fixture_uploaded=false")
                    throw SourceFailure.capabilityUnproven
                }
                try await Task.sleep(for: .milliseconds(500))
            }
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/brctl")
            process.arguments = ["evict", file.path]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else {
                print("offload_evict_command_passed=false")
                throw SourceFailure.capabilityUnproven
            }
            let evictionDeadline = Date().addingTimeInterval(15)
            while try cloudValues().ubiquitousItemDownloadingStatus != .notDownloaded {
                guard Date() < evictionDeadline else {
                    print("offload_fixture_evicted=false")
                    throw SourceFailure.capabilityUnproven
                }
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

actor CommandSink: ObservationSink {
    var rows: [JSON] = []
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { rows += observations }
}
