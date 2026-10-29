import Foundation
import Speech
import AVFoundation
import CoreMedia
import SourceSupport

public enum AudioQueries {
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        guard Set(args.object.keys).isSubset(of: ["file", "data_base64", "language"]), args["language"].string.utf8.count <= 64 else { throw SourceFailure.invalid }
        let data = try queryDocument(args)
        let locale = Locale(identifier: args["language"].string.isEmpty ? "en-US" : args["language"].string)
        if SpeechTranscriber.isAvailable, let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) {
            let transcriber = SpeechTranscriber(locale: supported, preset: .timeIndexedTranscriptionWithAlternatives)
            if await AssetInventory.status(forModules: [transcriber]) == .installed {
                return try await transcribe(data, transcriber: transcriber, language: supported.identifier)
            }
        }
        let status = SFSpeechRecognizer.authorizationStatus()
        let reason = status == .notDetermined ? "Speech Recognition authorization for the Helper is not_determined" : "On-device SpeechTranscriber assets are unavailable"
        return .object(["status": .string("unavailable"), "reason": .string(reason), "analyzer_reason": .string("SpeechTranscriber unavailable or locale assets not installed")])
    }
    private static func transcribe(_ data: Data, transcriber: SpeechTranscriber, language: String) async throws -> JSON {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("input.wav")
        try data.write(to: path)
        let file: AVAudioFile
        do { file = try AVAudioFile(forReading: path) } catch { throw SourceFailure.invalid }
        guard file.processingFormat.sampleRate > 0, Double(file.length) / file.processingFormat.sampleRate <= 120 else { throw SourceFailure.invalid }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        return try await withTaskCancellationHandler {
            async let segments = collect(transcriber)
            do {
                _ = try await analyzer.analyzeSequence(from: file)
                try await analyzer.finalizeAndFinishThroughEndOfInput()
                let result = try await segments
                return .object(["status": .string("available"), "text": .string(result.map { $0["text"].string }.joined(separator: " ")), "segments": .array(result), "language": .string(language)])
            } catch {
                await analyzer.cancelAndFinishNow()
                throw error
            }
        } onCancel: { Task { await analyzer.cancelAndFinishNow() } }
    }
    private static func collect(_ transcriber: SpeechTranscriber) async throws -> [JSON] {
        var segments: [JSON] = []
        var bytes = 0
        for try await result in transcriber.results {
            try Task.checkCancellation()
            let text = String(result.text.characters)
            bytes += text.utf8.count
            let start = result.range.start.seconds
            let end = CMTimeRangeGetEnd(result.range).seconds
            guard bytes <= 200000, segments.count < 10000, start.isFinite, end.isFinite, start >= 0, end >= start else { throw SourceFailure.invalid }
            segments.append(.object(["text": .string(text), "start": .number(start), "end": .number(end)]))
        }
        return segments
    }
}
