import Foundation
import CoreServices

public final class UsageWatcher: @unchecked Sendable {
    public let changes: AsyncStream<Void>
    private let continuation: AsyncStream<Void>.Continuation
    private let root: URL
    private var stream: FSEventStreamRef?
    public init(root: URL) {
        let root = usageRoot(root)
        self.root = root
        let pair = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream
        continuation = pair.continuation
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        stream = FSEventStreamCreate(nil, { _, info, count, paths, flags, _ in
            guard let info else { return }
            let watcher = Unmanaged<UsageWatcher>.fromOpaque(info).takeUnretainedValue()
            let names = Unmanaged<CFArray>.fromOpaque(paths).takeUnretainedValue() as! [String]
            let recovery = UInt32(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagUserDropped | kFSEventStreamEventFlagKernelDropped | kFSEventStreamEventFlagRootChanged)
            if (0..<count).contains(where: { flags[$0] & recovery != 0 || watcher.relevant(names[$0]) }) { watcher.continuation.yield(()) }
        }, &context, [root.appendingPathComponent("streams/restricted/App.InFocus").path, root.appendingPathComponent("sync").path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.1, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagNoDefer))
        if let stream {
            FSEventStreamSetDispatchQueue(stream, DispatchQueue(label: "applebridge.usage.events"))
            if !FSEventStreamStart(stream) { FSEventStreamInvalidate(stream); FSEventStreamRelease(stream); self.stream = nil }
        }
    }
    private func relevant(_ path: String) -> Bool {
        let streams = root.appendingPathComponent("streams/restricted/App.InFocus").path
        let sync = root.appendingPathComponent("sync").path
        return path == streams || path.hasPrefix(streams + "/") || path == sync || path == sync + "/sync.db" || path == sync + "/sync.db-wal"
    }
    deinit {
        if let stream { FSEventStreamStop(stream); FSEventStreamInvalidate(stream); FSEventStreamRelease(stream) }
        continuation.finish()
    }
}
