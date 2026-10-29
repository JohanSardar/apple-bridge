import Foundation
import CoreServices

public final class UsageWatcher: @unchecked Sendable {
    public let changes: AsyncStream<Void>
    private let continuation: AsyncStream<Void>.Continuation
    private var stream: FSEventStreamRef?
    public init(root: URL) {
        let pair = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream
        continuation = pair.continuation
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        stream = FSEventStreamCreate(nil, { _, info, _, _, _, _ in
            guard let info else { return }
            Unmanaged<UsageWatcher>.fromOpaque(info).takeUnretainedValue().continuation.yield(())
        }, &context, [root.appendingPathComponent("streams/restricted/App.InFocus").path, root.appendingPathComponent("sync").path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.1, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer))
        if let stream {
            FSEventStreamSetDispatchQueue(stream, DispatchQueue(label: "applebridge.usage.events"))
            if !FSEventStreamStart(stream) { FSEventStreamInvalidate(stream); FSEventStreamRelease(stream); self.stream = nil }
        }
    }
    deinit {
        if let stream { FSEventStreamStop(stream); FSEventStreamInvalidate(stream); FSEventStreamRelease(stream) }
        continuation.finish()
    }
}
