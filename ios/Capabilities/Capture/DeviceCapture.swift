import AVFoundation
import Foundation
import LocalAuthentication
import UIKit

@MainActor
final class DeviceCapture: NSObject, CaptureAdapter, AVCapturePhotoCaptureDelegate {
    let expires: Int64
    init(expires: Int64) { self.expires = expires }
    private var session: AVCaptureSession?
    private var photo: CheckedContinuation<CaptureBytes, Error>?

    func approve(kind: String, seconds: Int) async throws -> Bool {
        guard UIApplication.shared.applicationState == .active else { return false }
        return try await LAContext().evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, localizedReason: kind == "photo" ? "Approve one Apple Bridge photo" : "Approve \(seconds) seconds of Apple Bridge audio")
    }

    func capture(kind: String, seconds: Int) async throws -> CaptureBytes {
        guard UIApplication.shared.applicationState == .active, expires > Int64(Date().timeIntervalSince1970), UserDefaults.standard.bool(forKey: "capability.capture.enabled") else { throw CaptureError.denied }
        let media: AVMediaType = kind == "photo" ? .video : .audio
        guard await AVCaptureDevice.requestAccess(for: media), UIApplication.shared.applicationState == .active, expires > Int64(Date().timeIntervalSince1970), UserDefaults.standard.bool(forKey: "capability.capture.enabled") else { throw CaptureError.denied }
        if kind == "photo" { return try await takePhoto() }
        let audio = AVAudioSession.sharedInstance()
        try audio.setCategory(.record, mode: .default)
        try audio.setActive(true)
        defer { try? audio.setActive(false, options: .notifyOthersOnDeactivation) }
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else { throw CaptureError.invalid }
        let buffer = CaptureAudioBuffer(limit: Int(format.sampleRate) * seconds * 4)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { pcm, _ in
            guard let channel = pcm.floatChannelData?[0] else { return }
            buffer.append(Data(bytes: channel, count: Int(pcm.frameLength) * 4))
        }
        defer { input.removeTap(onBus: 0); engine.stop() }
        try engine.start()
        try await Task.sleep(for: .seconds(seconds))
        engine.stop()
        guard UIApplication.shared.applicationState == .active, expires > Int64(Date().timeIntervalSince1970), UserDefaults.standard.bool(forKey: "capability.capture.enabled") else { throw CaptureError.denied }
        return CaptureBytes(data: buffer.wav(rate: UInt32(format.sampleRate)), contentType: "audio/wav")
    }

    private func takePhoto() async throws -> CaptureBytes {
        let session = AVCaptureSession()
        session.sessionPreset = .photo
        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back) else { throw CaptureError.invalid }
        let input = try AVCaptureDeviceInput(device: device)
        let output = AVCapturePhotoOutput()
        guard session.canAddInput(input), session.canAddOutput(output) else { throw CaptureError.invalid }
        session.addInput(input)
        session.addOutput(output)
        self.session = session
        defer { session.stopRunning(); self.session = nil }
        session.startRunning()
        return try await withCheckedThrowingContinuation { continuation in
            photo = continuation
            output.capturePhoto(with: AVCapturePhotoSettings(format: [AVVideoCodecKey: AVVideoCodecType.jpeg]), delegate: self)
            Task {
                try? await Task.sleep(for: .seconds(15))
                self.photo?.resume(throwing: CaptureError.invalid)
                self.photo = nil
            }
        }
    }

    nonisolated func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
        let data = error == nil ? photo.fileDataRepresentation() : nil
        Task { @MainActor in
            if let data { self.photo?.resume(returning: CaptureBytes(data: data, contentType: "image/jpeg")) }
            else { self.photo?.resume(throwing: CaptureError.invalid) }
            self.photo = nil
        }
    }
}

final class CaptureAudioBuffer: @unchecked Sendable {
    private let lock = NSLock()
    private var data = Data()
    private let limit: Int
    init(limit: Int) { self.limit = limit }
    func append(_ bytes: Data) {
        lock.lock()
        defer { lock.unlock() }
        data.append(bytes.prefix(max(0, limit - data.count)))
    }
    func wav(rate: UInt32) -> Data {
        lock.lock()
        defer { lock.unlock(); data.resetBytes(in: 0..<data.count); data.removeAll() }
        var result = Data("RIFF".utf8)
        func number<T: FixedWidthInteger>(_ value: T) { var n = value.littleEndian; withUnsafeBytes(of: &n) { result.append(contentsOf: $0) } }
        number(UInt32(36 + data.count))
        result.append(Data("WAVEfmt ".utf8))
        number(UInt32(16)); number(UInt16(3)); number(UInt16(1)); number(rate); number(rate * 4); number(UInt16(4)); number(UInt16(32))
        result.append(Data("data".utf8)); number(UInt32(data.count)); result.append(data)
        return result
    }
}
