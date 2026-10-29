import Foundation
import CryptoKit
import CoreGraphics
import AVFoundation
import PDFKit
import ScreenCaptureKit
import ImageIO
import UniformTypeIdentifiers
import SourceSupport

public struct CaptureArtifact: Sendable {
    public let url: URL
    public let duration: Double
    public let width: Int
    public let height: Int
    public init(url: URL, duration: Double = 0, width: Int = 0, height: Int = 0) {
        self.url = url; self.duration = duration; self.width = width; self.height = height
    }
}

public protocol CaptureProvider: Sendable {
    func authorized(_ operation: String) async -> Bool
    func produce(_ operation: String, args: JSON, directory: URL) async throws -> CaptureArtifact
}

public actor CaptureSource: Source {
    public nonisolated let name = "capture"
    public static let operations = ["capture.screenshot", "capture.camera_photo", "capture.record_audio", "mac.screen_record", "print.document"]
    let provider: any CaptureProvider
    let directory: URL
    var artifacts: [String: CaptureArtifact] = [:]
    public init(directory: URL, provider: any CaptureProvider = SystemCaptureProvider()) {
        self.directory = directory; self.provider = provider
    }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("partial")]), capabilities: Self.operations)
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {}
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        let op = action["op"].string
        let args = action["args"]
        guard Self.operations.contains(op), case .object = args else { throw SourceFailure.invalid }
        let allowed: Set<String>
        switch op {
        case "capture.screenshot":
            allowed = ["display", "window"]
            guard args["display"] == .null || args["window"] == .null else { throw SourceFailure.invalid }
            for value in args.object.values { guard value.number > 0, value.number.rounded() == value.number else { throw SourceFailure.invalid } }
        case "capture.camera_photo": allowed = []
        case "capture.record_audio", "mac.screen_record":
            allowed = ["seconds"]
            let seconds = args["seconds"].number
            guard seconds >= 1, seconds <= (op == "mac.screen_record" ? 30 : 60), seconds.isFinite else { throw SourceFailure.invalid }
        default:
            allowed = ["file", "printer"]
            guard !args["file"].string.isEmpty, args["printer"].string == "PDF" else { throw SourceFailure.unsupported }
        }
        guard Set(args.object.keys).isSubset(of: allowed) else { throw SourceFailure.invalid }
        guard await provider.authorized(op) else { throw SourceFailure.permissionMissing }
        return Prepared(nativeState: .null, fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        _ = try await prepare(.object(["action": prepared.action]))
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let artifact = try await provider.produce(prepared.action["op"].string, args: prepared.action["args"], directory: directory)
        let evidence = try verify(artifact, action: prepared.action)
        artifacts[evidence["digest"].string] = artifact
        return evidence
    }
    func verify(_ artifact: CaptureArtifact, action: JSON) throws -> JSON {
        guard artifact.url.deletingLastPathComponent().standardizedFileURL == directory.standardizedFileURL else { throw SourceFailure.corrupt }
        let values = try artifact.url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true, let size = values.fileSize, size > 0, size <= 128 << 20 else { throw SourceFailure.corrupt }
        guard artifact.duration.isFinite, artifact.duration >= 0 else { throw SourceFailure.corrupt }
        let seconds = action["args"]["seconds"].number
        guard seconds == 0 || abs(artifact.duration - seconds) <= 0.5 else { throw SourceFailure.corrupt }
        if action["op"].string == "capture.screenshot" { guard artifact.width > 0, artifact.height > 0 else { throw SourceFailure.corrupt } }
        let bytes = try Data(contentsOf: artifact.url)
        guard bytes.count == size else { throw SourceFailure.corrupt }
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        return .object(["digest": .string(digest), "size": .number(Double(size)), "duration": .number(artifact.duration), "width": .number(Double(artifact.width)), "height": .number(Double(artifact.height)), "observed": .bool(true)])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard let artifact = artifacts[evidence["digest"].string], let verified = try? verify(artifact, action: evidence["prepared"]["action"]), verified["digest"] == evidence["digest"], verified["size"] == evidence["size"] else { return .null }
        return verified
    }
}

public struct SystemCaptureProvider: CaptureProvider {
    public init() {}
    public func authorized(_ operation: String) async -> Bool {
        switch operation {
        case "capture.screenshot", "mac.screen_record": return CGPreflightScreenCaptureAccess()
        case "capture.camera_photo": return AVCaptureDevice.authorizationStatus(for: .video) == .authorized
        case "capture.record_audio": return AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
        default: return true
        }
    }
    public func produce(_ operation: String, args: JSON, directory: URL) async throws -> CaptureArtifact {
        if operation == "capture.screenshot" {
            guard CGPreflightScreenCaptureAccess() else { throw SourceFailure.permissionMissing }
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            let filter: SCContentFilter
            let configuration = SCStreamConfiguration()
            if args["window"] != .null {
                guard let window = content.windows.first(where: { Double($0.windowID) == args["window"].number }) else { throw SourceFailure.unavailable }
                filter = SCContentFilter(desktopIndependentWindow: window)
                configuration.width = Int(window.frame.width * 2)
                configuration.height = Int(window.frame.height * 2)
            } else {
                let identifier = args["display"] == .null ? Double(CGMainDisplayID()) : args["display"].number
                guard let display = content.displays.first(where: { Double($0.displayID) == identifier }) else { throw SourceFailure.unavailable }
                filter = SCContentFilter(display: display, excludingWindows: [])
                configuration.width = display.width
                configuration.height = display.height
            }
            configuration.showsCursor = false
            let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
            let output = directory.appendingPathComponent(UUID().uuidString + ".png")
            guard let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil) else { throw SourceFailure.unavailable }
            CGImageDestinationAddImage(destination, image, nil)
            guard CGImageDestinationFinalize(destination) else { throw SourceFailure.unavailable }
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
            return CaptureArtifact(url: output, width: image.width, height: image.height)
        }
        if operation == "capture.record_audio" {
            guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else { throw SourceFailure.permissionMissing }
            let output = directory.appendingPathComponent(UUID().uuidString + ".m4a")
            let recorder = try AVAudioRecorder(url: output, settings: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44100, AVNumberOfChannelsKey: 1, AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue])
            defer { recorder.stop() }
            guard recorder.record(forDuration: args["seconds"].number) else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .seconds(args["seconds"].number + 0.1))
            recorder.stop()
            let duration = try await AVURLAsset(url: output).load(.duration).seconds
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
            return CaptureArtifact(url: output, duration: duration)
        }
        guard operation == "print.document" else { throw SourceFailure.unsupported }
        let input = URL(fileURLWithPath: args["file"].string)
        guard let document = PDFDocument(url: input), document.pageCount > 0, let bytes = document.dataRepresentation() else { throw SourceFailure.invalid }
        let output = directory.appendingPathComponent(UUID().uuidString + ".pdf")
        try bytes.write(to: output, options: [.atomic])
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
        guard PDFDocument(url: output)?.pageCount == document.pageCount else { throw SourceFailure.corrupt }
        return CaptureArtifact(url: output)
    }
}
