import Foundation
import Darwin
import CoreGraphics
import ImageIO
import PDFKit
import FilesSource
import SourceSupport

@main
struct Checks {
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
