import Foundation
import AVFoundation
import AppKit
import SourceSupport

public actor SpeechPlayback: Source {
    public nonisolated let name: String
    public init(name: String = "speech") { self.name = name }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete")]), capabilities: [name == "speech" ? "speech.say" : "capture.play_sound"])
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        try await Task.sleep(for: .seconds(86400))
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        let args = action["args"]
        guard action["op"].string == (name == "speech" ? "speech.say" : "capture.play_sound"), Set(args.object.keys).isSubset(of: ["text", "name", "volume"]), args["volume"] == .null || args["volume"].number >= 0 && args["volume"].number <= 1 else { throw SourceFailure.invalid }
        if name == "speech" {
            guard !args["text"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, args["text"].string.utf8.count <= 2048, args["name"].string.isEmpty else { throw SourceFailure.invalid }
        } else {
            guard Self.sounds.contains(args["name"].string), args["text"].string.isEmpty else { throw SourceFailure.invalid }
        }
        return Prepared(nativeState: .object(["route": .string("playback")]), fingerprint: command["request_hash"].string, action: action)
    }
    private static let sounds: Set<String> = ["Basso", "Blow", "Bottle", "Frog", "Funk", "Glass", "Hero", "Morse", "Ping", "Pop", "Purr", "Sosumi", "Submarine", "Tink"]
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let finished = try await Self.play(prepared.action)
        return .object(["disposition": .string(finished ? "locally_observed" : "indeterminate"), "finished_callback": .bool(finished), "observed": .bool(finished)])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard evidence["finished_callback"].bool, !evidence["prepared"]["request_hash"].string.isEmpty else { return .null }
        var value = evidence.object
        value["observed"] = .bool(true)
        return .object(value)
    }
    @MainActor
    private static func play(_ action: JSON) async throws -> Bool {
        let args = action["args"]
        let delegate = PlaybackCompletion()
        let deadline = ContinuousClock.now.advanced(by: .seconds(120))
        let volume = Float(args["volume"] == .null ? 1 : args["volume"].number)
        if action["op"].string == "speech.say" {
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
            guard let sound = NSSound(contentsOfFile: "/System/Library/Sounds/" + args["name"].string + ".aiff", byReference: false) else { throw SourceFailure.unavailable }
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
