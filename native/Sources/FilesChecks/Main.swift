import Foundation
import CoreGraphics
import ImageIO
import AppKit
import PDFKit
import FilesSource
import SourceSupport

@main
struct Checks {
    static func transforms(_ source: FilesSource, directory: URL) async throws {
        guard let context = CGContext(data: nil, width: 16, height: 12, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue), let image = context.makeImage() else { throw SourceFailure.corrupt }
        let png = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(png, "public.png" as CFString, 1, nil) else { throw SourceFailure.corrupt }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { throw SourceFailure.corrupt }
        let staged = directory.appendingPathComponent("staged")
        func run(_ fields: [String: JSON]) async throws -> [JSON] {
            var args = fields
            args["directory"] = .string(staged.path)
            return try await source.primitive("files.transform", args: .object(args))["outputs"].array
        }
        let imageFile = directory.appendingPathComponent("image.png")
        try (png as Data).write(to: imageFile)
        for operation in ["convert", "resize"] {
            let outputs = try await run(["operation": .string(operation), "inputs": .array([.string(imageFile.path)]), "format": .string("jpeg"), "width": .number(8), "height": .number(6)])
            guard outputs.count == 1, outputs[0]["media_type"].string == "image/jpeg", let decoded = CGImageSourceCreateWithData(try Data(contentsOf: URL(fileURLWithPath: outputs[0]["file"].string)) as CFData, nil), CGImageSourceGetType(decoded) as String? == "public.jpeg" else { throw SourceFailure.corrupt }
        }
        let pdf = PDFDocument()
        guard let page = PDFPage(image: NSImage(cgImage: image, size: .zero)) else { throw SourceFailure.corrupt }
        pdf.insert(page, at: 0)
        let pdfFile = directory.appendingPathComponent("one.pdf")
        try pdf.dataRepresentation()!.write(to: pdfFile)
        let combined = try await run(["operation": .string("combine"), "inputs": .array([.string(pdfFile.path), .string(pdfFile.path)])])
        guard combined.count == 1, PDFDocument(url: URL(fileURLWithPath: combined[0]["file"].string))?.pageCount == 2 else { throw SourceFailure.corrupt }
        let split = try await run(["operation": .string("split"), "inputs": .array([combined[0]["file"]])])
        guard split.count == 2 else { throw SourceFailure.corrupt }
        for blob in split {
            guard blob["media_type"].string == "application/pdf", PDFDocument(url: URL(fileURLWithPath: blob["file"].string))?.pageCount == 1 else { throw SourceFailure.corrupt }
        }
        do {
            _ = try await run(["operation": .string("combine"), "inputs": .array([.string(imageFile.path)])])
            throw SourceFailure.corrupt
        } catch SourceFailure.invalid {}
        print("transforms=4 passed=true")
    }
    static func actions(_ source: FilesSource, directory: URL) async throws {
        let file = directory.appendingPathComponent("a.txt")
        try Data("a".utf8).write(to: file)
        let path = JSON.string(file.path)
        let answer = source.ubiquity([file.path, directory.appendingPathComponent("gone.txt").path])
        guard answer.count == 2, answer[0]["path"].string == file.path, answer[0]["status"].string == "local", answer[1]["status"].string == "local" else { throw SourceFailure.corrupt }
        let state = try await source.primitive("files.cloud_state", args: .object(["path": path]))
        guard state["ubiquitous"] == .bool(false) else { throw SourceFailure.corrupt }
        let tagged = try await source.primitive("files.set_tags", args: .object(["path": path, "tags": .array([.string("Bridge Test")])]))
        guard tagged["tags"] == .array([.string("Bridge Test")]) else { throw SourceFailure.corrupt }
        let staged = try await source.primitive("files.stage", args: .object(["path": path, "directory": .string(directory.appendingPathComponent("staged").path)]))
        guard staged["digest"].string == "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4c72c9201c9ab3f4e7c1" || staged["size"].number == 1, FileManager.default.fileExists(atPath: staged["file"].string) else { throw SourceFailure.corrupt }
        do {
            _ = try await source.primitive("files.stage", args: .object(["path": .string(directory.appendingPathComponent("gone.txt").path), "directory": .string(directory.path)]))
            throw SourceFailure.corrupt
        } catch SourceFailure.corrupt { throw SourceFailure.corrupt } catch {}
        do {
            _ = try await source.primitive("files.other", args: .object([:]))
            throw SourceFailure.corrupt
        } catch SourceFailure.unsupported {}
        print("actions passed=true")
    }
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-" + UUID().uuidString).resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = FilesSource()
        try await actions(source, directory: directory)
        try await transforms(source, directory: directory)
    }
}
