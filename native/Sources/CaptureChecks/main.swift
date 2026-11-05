import Foundation
import CaptureSource
import SourceSupport
import PDFKit

actor Fake: CaptureProvider {
    var allowed = true
    func deny() { allowed = false }
    func authorized(_ operation: String) async -> Bool { allowed }
    func produce(_ operation: String, args: JSON, directory: URL) async throws -> CaptureArtifact {
        let url = directory.appendingPathComponent("[ab-test] capture " + UUID().uuidString)
        try Data([1, 2, 3]).write(to: url)
        return CaptureArtifact(url: url, duration: args["seconds"].number, width: 2, height: 2)
    }
}

@main struct Checks {
    @MainActor static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let provider = Fake()
        let source = CaptureSource(provider: provider)
        let produced = try await source.primitive("capture.produce", args: .object(["op": .string("capture.record_audio"), "args": .object(["seconds": .number(2)]), "directory": .string(directory.path)]))
        guard produced["duration"] == .number(2), produced["width"] == .number(2), FileManager.default.fileExists(atPath: produced["path"].string) else { throw SourceFailure.corrupt }
        guard try await source.primitive("capture.authorized", args: .object(["op": .string("capture.camera_photo")]))["authorized"] == .bool(true) else { throw SourceFailure.corrupt }
        await provider.deny()
        guard try await source.primitive("capture.authorized", args: .object(["op": .string("capture.camera_photo")]))["authorized"] == .bool(false) else { throw SourceFailure.corrupt }
        guard case .array = try await source.primitive("capture.printers", args: .object([:]))["printers"] else { throw SourceFailure.corrupt }
        do { _ = try await source.primitive("capture.other", args: .object([:])); throw SourceFailure.corrupt } catch SourceFailure.unsupported {}
        let pdf = PDFDocument()
        pdf.insert(PDFPage(), at: 0)
        let input = directory.appendingPathComponent("input.pdf")
        guard pdf.write(to: input) else { throw SourceFailure.corrupt }
        let printed = try await CaptureSource().primitive("capture.produce", args: .object(["op": .string("print.document"), "args": .object(["file": .string(input.path), "printer": .string("PDF")]), "directory": .string(directory.path)]))
        guard PDFDocument(url: URL(fileURLWithPath: printed["path"].string))?.pageCount == 1 else { throw SourceFailure.corrupt }
        print("CAP-SIM provider lifecycle passed")
    }
}
