import Foundation
import SourceSupport

@main
struct Checks {
    static func require(_ value: Bool) throws {
        guard value else { throw SourceFailure.invalid }
    }
    static func rejects(_ action: () throws -> Void) throws {
        do { try action() } catch { return }
        throw SourceFailure.invalid
    }
    static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = try BlobWriter(directory: root.appendingPathComponent("blobs"))
        let blob = try writer.stage(Data("abc".utf8))
        try require(blob.digest == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        let staged = try Data(contentsOf: blob.file)
        try require(blob.size == 3 && staged == Data("abc".utf8))
        let copy = try writer.stage(blob.file)
        try require(copy.digest == blob.digest)
        let large = Data(repeating: 42, count: 400_000)
        let largeBlob = try writer.stage(large)
        let largeCopy = try writer.stage(largeBlob.file)
        try require(largeCopy.digest == largeBlob.digest && largeCopy.size == Int64(large.count))
        let link = root.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: blob.file)
        try rejects { _ = try writer.stage(link) }
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try writer.stage(Data("cancelled".utf8))
        }
        do { _ = try await cancelled.value; throw SourceFailure.invalid }
        catch is CancellationError {}
        try require(await AppleScriptRunner.automationStatus { 0 } == 0)
        try require(await AppleScriptRunner.automationStatus { -1743 } == -1743)
        let release = DispatchSemaphore(value: 0)
        let started = ContinuousClock.now
        try require(await AppleScriptRunner.automationStatus(after: 0.02) { _ = release.wait(timeout: .now() + 2); return 0 } == -1712)
        release.signal()
        try require(ContinuousClock.now - started < .seconds(1))
        print("all SourceSupport checks passed")
    }
}
