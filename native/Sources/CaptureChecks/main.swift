import Foundation
import CaptureSource
import SourceSupport
import PDFKit

actor Fake: CaptureProvider {
    var allowed = true
    var broken = false
    var wrongDuration = false
    func changeDuration() { wrongDuration = true }
    var calls = 0
    var last: URL?
    func removeLast() throws { if let last { try FileManager.default.removeItem(at: last) } }
    func deny() { allowed = false }
    func breakOutput() { broken = true }
    func count() -> Int { calls }
    func outputExists() -> Bool { last.map { FileManager.default.fileExists(atPath: $0.path) } ?? false }
    func authorized(_ operation: String) async -> Bool { allowed }
    func produce(_ operation: String, args: JSON, directory: URL) async throws -> CaptureArtifact {
        calls += 1
        let url = directory.appendingPathComponent(UUID().uuidString)
        try Data(broken ? [] : [1, 2, 3]).write(to: url)
        last = url
        return CaptureArtifact(url: url, duration: wrongDuration ? 100 : args["seconds"].number, width: 2, height: 2)
    }
}

@main struct Checks {
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let provider = Fake()
        let source = CaptureSource(directory: directory, provider: provider)
        for op in CaptureSource.operations {
            var args: JSON = .object([:])
            if op == "capture.record_audio" || op == "mac.screen_record" { args = .object(["seconds": .number(2)]) }
            if op == "print.document" { args = .object(["file": .string("fixture.pdf"), "printer": .string("PDF")]) }
            let action: JSON = .object(["op": .string(op), "args": args])
            let prepared = try await source.prepare(.object(["action": action, "request_hash": .string("synthetic")]))
            let evidence = try await source.dispatch(prepared)
            guard evidence["size"] == .number(3), evidence["digest"].string.count == 64, evidence.object["data"] == nil else { throw SourceFailure.corrupt }
            var saved = evidence.object
            saved["prepared"] = .object(["action": action])
            guard await source.resolve("synthetic", evidence: .object(saved))["observed"].bool else { throw SourceFailure.corrupt }
            try await provider.removeLast()
            guard await source.resolve("synthetic", evidence: .object(saved)) == .null else { throw SourceFailure.corrupt }
        }
        for (op, seconds) in [("capture.record_audio", 61.0), ("mac.screen_record", 31.0)] {
            do {
                _ = try await source.prepare(.object(["action": .object(["op": .string(op), "args": .object(["seconds": .number(seconds)])])]))
                throw SourceFailure.corrupt
            } catch SourceFailure.invalid {}
        }
        let action: JSON = .object(["op": .string("capture.camera_photo"), "args": .object([:])])
        let prepared = try await source.prepare(.object(["action": action]))
        await provider.deny()
        do { _ = try await source.dispatch(prepared); throw SourceFailure.corrupt } catch SourceFailure.permissionMissing {}
        guard await provider.count() == 5 else { throw SourceFailure.corrupt }
        let broken = Fake()
        await broken.breakOutput()
        let other = CaptureSource(directory: directory, provider: broken)
        let ticket = try await other.prepare(.object(["action": action]))
        do { _ = try await other.dispatch(ticket); throw SourceFailure.invalid } catch SourceFailure.corrupt {}
        guard !(await broken.outputExists()) else { throw SourceFailure.corrupt }
        let mistimed = Fake()
        await mistimed.changeDuration()
        let timed = CaptureSource(directory: directory, provider: mistimed)
        let timedAction: JSON = .object(["op": .string("capture.record_audio"), "args": .object(["seconds": .number(2)])])
        let timedTicket = try await timed.prepare(.object(["action": timedAction]))
        do { _ = try await timed.dispatch(timedTicket); throw SourceFailure.invalid } catch SourceFailure.corrupt {}
        guard !(await mistimed.outputExists()) else { throw SourceFailure.corrupt }
        let pdf = PDFDocument()
        let page = PDFPage()
        pdf.insert(page, at: 0)
        let input = directory.appendingPathComponent("input.pdf")
        guard pdf.write(to: input) else { throw SourceFailure.corrupt }
        let real = CaptureSource(directory: directory)
        let printAction: JSON = .object(["op": .string("print.document"), "args": .object(["file": .string(input.path), "printer": .string("PDF")])])
        let printTicket = try await real.prepare(.object(["action": printAction]))
        let printed = try await real.dispatch(printTicket)
        guard printed["size"].number > 0 else { throw SourceFailure.corrupt }
        print("CAP-SIM provider lifecycle passed")
    }
}
