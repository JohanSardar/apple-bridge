import AppKit
import SpeechSource
import Foundation
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

public struct CaptureSource: Source {
    public let name = "capture"
    let provider: any CaptureProvider
    let playback = SpeechPlayback(name: "capture")
    public init(provider: any CaptureProvider = SystemCaptureProvider()) {
        self.provider = provider
    }
    public func status() async -> JSON? { .object([:]) }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        switch operation {
        case "capture.play_sound":
            return try await playback.primitive(operation, args: args)
        case "capture.authorized":
            return .object(["authorized": .bool(await provider.authorized(args["op"].string))])
        case "capture.printers":
            return .object(["printers": .array(NSPrinter.printerNames.map(JSON.string))])
        case "capture.produce":
            let artifact = try await provider.produce(args["op"].string, args: args["args"], directory: URL(fileURLWithPath: args["directory"].string))
            return .object(["path": .string(artifact.url.path), "duration": .number(artifact.duration), "width": .number(Double(artifact.width)), "height": .number(Double(artifact.height))])
        default:
            throw SourceFailure.unsupported
        }
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
        if operation == "mac.screen_record" { return try await screenVideo(seconds: args["seconds"].number, directory: directory) }
        if operation == "capture.camera_photo" { return try await cameraPhoto(directory: directory) }
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
