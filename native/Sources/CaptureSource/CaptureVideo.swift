import Foundation
import AVFoundation
import ScreenCaptureKit
import CoreGraphics
import CoreImage
import SourceSupport

final class RecordingCompletion: NSObject, SCRecordingOutputDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var complete = false
    private var started = false
    private var failure: (any Error)?
    func recordingOutputDidStartRecording(_ recordingOutput: SCRecordingOutput) {
        lock.withLock { started = true }
    }
    func waitForStart() async throws {
        let deadline = Date().addingTimeInterval(10)
        while true {
            let state = lock.withLock { (started, failure) }
            if let error = state.1 { throw error }
            if state.0 { return }
            guard Date() < deadline else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
    func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) {
        lock.withLock { complete = true }
    }
    func recordingOutput(_ recordingOutput: SCRecordingOutput, didFailWithError error: any Error) {
        lock.withLock { failure = error; complete = true }
    }
    func wait() async throws {
        let deadline = Date().addingTimeInterval(10)
        while true {
            let state = lock.withLock { (complete, failure) }
            if let error = state.1 { throw error }
            if state.0 { return }
            guard Date() < deadline else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}

final class CameraFrame: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var frame: CVPixelBuffer?
    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let image = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        lock.withLock { if frame == nil { frame = image } }
    }
    func image() -> CVPixelBuffer? { lock.withLock { frame } }
}

extension SystemCaptureProvider {
    func screenVideo(seconds: Double, directory: URL) async throws -> CaptureArtifact {
        guard CGPreflightScreenCaptureAccess() else { throw SourceFailure.permissionMissing }
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() }) else { throw SourceFailure.unavailable }
        let configuration = SCStreamConfiguration()
        configuration.width = display.width
        configuration.height = display.height
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 30)
        configuration.showsCursor = false
        let output = directory.appendingPathComponent(UUID().uuidString + ".mp4")
        var succeeded = false
        defer { if !succeeded { try? FileManager.default.removeItem(at: output) } }
        let recordingConfiguration = SCRecordingOutputConfiguration()
        recordingConfiguration.outputURL = output
        recordingConfiguration.outputFileType = .mp4
        recordingConfiguration.videoCodecType = .h264
        let completion = RecordingCompletion()
        let recording = SCRecordingOutput(configuration: recordingConfiguration, delegate: completion)
        let stream = SCStream(filter: SCContentFilter(display: display, excludingWindows: []), configuration: configuration, delegate: nil)
        try stream.addRecordingOutput(recording)
        try await stream.startCapture()
        do {
            try await completion.waitForStart()
            try await Task.sleep(for: .seconds(seconds))
            try await stream.stopCapture()
            try await completion.wait()
        } catch {
            try? await stream.stopCapture()
            throw error
        }
        let duration = try await AVURLAsset(url: output).load(.duration).seconds
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
        succeeded = true
        return CaptureArtifact(url: output, duration: duration, width: display.width, height: display.height)
    }

    func cameraPhoto(directory: URL) async throws -> CaptureArtifact {
        guard AVCaptureDevice.authorizationStatus(for: .video) == .authorized else { throw SourceFailure.permissionMissing }
        guard let device = AVCaptureDevice.default(for: .video) else { throw SourceFailure.unavailable }
        let session = AVCaptureSession()
        let input = try AVCaptureDeviceInput(device: device)
        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        let receiver = CameraFrame()
        output.setSampleBufferDelegate(receiver, queue: DispatchQueue(label: "apple-bridge-camera"))
        guard session.canAddInput(input), session.canAddOutput(output) else { throw SourceFailure.unavailable }
        session.addInput(input)
        session.addOutput(output)
        session.startRunning()
        defer { session.stopRunning() }
        let deadline = Date().addingTimeInterval(10)
        while receiver.image() == nil {
            guard Date() < deadline else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard let frame = receiver.image() else { throw SourceFailure.unavailable }
        let image = CIImage(cvPixelBuffer: frame)
        let url = directory.appendingPathComponent(UUID().uuidString + ".jpg")
        do {
            try CIContext().writeJPEGRepresentation(of: image, to: url, colorSpace: CGColorSpaceCreateDeviceRGB())
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            try? FileManager.default.removeItem(at: url)
            throw error
        }
        return CaptureArtifact(url: url, width: CVPixelBufferGetWidth(frame), height: CVPixelBufferGetHeight(frame))
    }
}
