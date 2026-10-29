import Foundation
import AVFoundation
import SourceSupport

public enum SpeechQueries {
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        guard Set(args.object.keys).isSubset(of: ["text"]), !args["text"].string.isEmpty, args["text"].string.utf8.count <= 2048 else { throw SourceFailure.invalid }
        return try await synthesize(args["text"].string)
    }
    @MainActor
    private static func synthesize(_ text: String) async throws -> JSON {
        let synth = AVSpeechSynthesizer()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("speech.wav")
        let output = SpeechOutput(path)
        synth.write(AVSpeechUtterance(string: text)) { buffer in output.accept(buffer) }
        defer { synth.stopSpeaking(at: .immediate) }
        while !output.finished {
            try await Task.sleep(for: .milliseconds(20))
        }
        guard !output.failed else { throw SourceFailure.unavailable }
        let data = try Data(contentsOf: path)
        guard data.count <= 700000 else { throw SourceFailure.invalid }
        return .object(["media_type": .string("audio/wav"), "data_base64": .string(data.base64EncodedString())])
    }
}

private final class SpeechOutput: @unchecked Sendable {
    private let lock = NSLock()
    private let path: URL
    private var file: AVAudioFile?
    private var done = false
    private var failure = false
    init(_ path: URL) { self.path = path }
    var failed: Bool { lock.lock(); defer { lock.unlock() }; return failure }
    var finished: Bool { lock.lock(); defer { lock.unlock() }; return done }
    func accept(_ buffer: AVAudioBuffer) {
        lock.lock()
        defer { lock.unlock() }
        guard !done, let pcm = buffer as? AVAudioPCMBuffer else { return }
        if pcm.frameLength == 0 { done = true; file = nil; return }
        do {
            if file == nil { file = try AVAudioFile(forWriting: path, settings: pcm.format.settings) }
            try file?.write(from: pcm)
            if let size = try? path.resourceValues(forKeys: [.fileSizeKey]).fileSize, size > 700000 { done = true; file = nil }
        } catch { failure = true; done = true; file = nil }
    }
}
