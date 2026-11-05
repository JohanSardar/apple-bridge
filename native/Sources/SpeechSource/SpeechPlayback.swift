import Foundation
import AVFoundation
import AppKit
import SourceSupport

public actor SpeechPlayback: Source {
    public nonisolated let name: String
    public init(name: String = "speech") { self.name = name }
    public func status() async -> JSON? { .object([:]) }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard operation == "speech.say" || operation == "capture.play_sound" else { throw SourceFailure.unsupported }
        return .object(["finished": .bool(try await Self.play(operation, args))])
    }
    @MainActor
    private static func play(_ operation: String, _ args: JSON) async throws -> Bool {
        let delegate = PlaybackCompletion()
        let deadline = ContinuousClock.now.advanced(by: .seconds(120))
        let volume = Float(args["volume"].number)
        if operation == "speech.say" {
            let synth = AVSpeechSynthesizer()
            synth.delegate = delegate
            let utterance = AVSpeechUtterance(string: args["text"].string)
            utterance.volume = volume
            synth.speak(utterance)
            defer { synth.stopSpeaking(at: .immediate) }
            while !delegate.finished {
                guard ContinuousClock.now < deadline else { return false }
                try await Task.sleep(for: .milliseconds(20))
            }
        } else {
            guard let sound = NSSound(contentsOfFile: args["path"].string, byReference: false) else { throw SourceFailure.unavailable }
            sound.volume = volume
            sound.delegate = delegate
            guard sound.play() else { throw SourceFailure.unavailable }
            defer { sound.stop() }
            while !delegate.finished {
                guard ContinuousClock.now < deadline else { return false }
                try await Task.sleep(for: .milliseconds(20))
            }
        }
        return delegate.success
    }
}

private final class PlaybackCompletion: NSObject, AVSpeechSynthesizerDelegate, NSSoundDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    private var succeeded = false
    var finished: Bool { lock.lock(); defer { lock.unlock() }; return done }
    var success: Bool { lock.lock(); defer { lock.unlock() }; return succeeded }
    private func finish(_ success: Bool) { lock.lock(); defer { lock.unlock() }; done = true; succeeded = success }
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) { finish(true) }
    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) { finish(false) }
    func sound(_ sound: NSSound, didFinishPlaying flag: Bool) { finish(flag) }
}
