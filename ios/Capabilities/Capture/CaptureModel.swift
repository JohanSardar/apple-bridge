import Foundation

struct CaptureBytes {
    var data: Data
    let contentType: String
}

@MainActor
protocol CaptureAdapter {
    func approve(kind: String, seconds: Int) async throws -> Bool
    func capture(kind: String, seconds: Int) async throws -> CaptureBytes
}

@MainActor
enum CaptureFlow {
    static func run(kind: String, seconds: Int, adapter: any CaptureAdapter, upload: (CaptureBytes) async throws -> Data) async throws -> Data {
        guard ["photo", "audio"].contains(kind), (1...10).contains(seconds), try await adapter.approve(kind: kind, seconds: seconds) else { throw CaptureError.denied }
        var bytes = try await adapter.capture(kind: kind, seconds: seconds)
        defer { bytes.data.resetBytes(in: 0..<bytes.data.count); bytes.data.removeAll(keepingCapacity: false) }
        guard !bytes.data.isEmpty, bytes.data.count <= 12 * 1024 * 1024 else { throw CaptureError.invalid }
        return try await upload(bytes)
    }
}

enum CaptureError: Error { case denied, invalid }
