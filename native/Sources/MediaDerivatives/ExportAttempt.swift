import Foundation
import AVFoundation
import MediaExportGuard

final class ExportAttempt: @unchecked Sendable {
    private let lock = NSLock()
    private let session: AVAssetExportSession
    private var cancelled = false
    private var started = false
    init(_ session: AVAssetExportSession) { self.session = session }
    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }
    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        cancelled = true
        if started { session.cancelExport() }
    }
    func start(_ url: URL, type: AVFileType, continuation: CheckedContinuation<Void, any Error>) {
        lock.lock()
        defer { lock.unlock() }
        guard !cancelled else { continuation.resume(throwing: CancellationError()); return }
        let completion = ExportCompletion(continuation)
        if let error = ABMediaExportGuard.start(session, outputURL: url, fileType: type, completion: { error in
            if let error, (error as NSError).domain == NSCocoaErrorDomain && (error as NSError).code == NSUserCancelledError { completion.finish(CancellationError()) }
            else { completion.finish(error) }
        }) { completion.finish(error) }
        else { started = true }
    }
}

private final class ExportCompletion: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Void, any Error>?
    init(_ continuation: CheckedContinuation<Void, any Error>) { self.continuation = continuation }
    func finish(_ error: (any Error)?) {
        lock.lock()
        let current = continuation
        continuation = nil
        lock.unlock()
        if let error { current?.resume(throwing: error) } else { current?.resume() }
    }
}

func exportFailure(_ reason: String) -> NSError {
    NSError(domain: "AppleBridgeMediaExport", code: 1, userInfo: ["reason": reason])
}

func safeExportReason(_ error: any Error, input: URL, directory: URL) -> String {
    let failure = error as NSError
    let name = failure.userInfo["exception_name"] as? String ?? "ExportUnavailable"
    var reason = failure.userInfo["reason"] as? String ?? "Export failed"
    for url in [input, directory] {
        for value in [url.absoluteString, url.path, url.lastPathComponent] where !value.isEmpty { reason = reason.replacingOccurrences(of: value, with: "[redacted]") }
    }
    reason = reason.replacingOccurrences(of: #"(?:file:|https?:)\S+|/[^\s\"']+"#, with: "[redacted]", options: .regularExpression)
    let summary = name + ": " + String(reason.prefix(512))
    try? FileHandle.standardError.write(contentsOf: Data((summary + "\n").utf8))
    return summary
}
