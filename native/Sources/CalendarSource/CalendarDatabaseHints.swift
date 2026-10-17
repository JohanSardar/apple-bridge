import Foundation
import CoreServices
import SourceSupport

final class CalendarDatabaseHints: @unchecked Sendable {
    let changes: AsyncStream<UInt64>
    private let continuation: AsyncStream<UInt64>.Continuation
    private let queue = DispatchQueue(label: "applebridge.calendar.hints")
    private let paths: Set<String>
    private var events: FSEventStreamRef?

    init() {
        let database = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb").path
        paths = [database, database + "-wal"]
        let pair = AsyncStream<UInt64>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream
        continuation = pair.continuation
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        events = FSEventStreamCreate(nil, { _, info, count, rawPaths, flags, _ in
            guard let info else { return }
            let watcher = Unmanaged<CalendarDatabaseHints>.fromOpaque(info).takeUnretainedValue()
            let changed = unsafeBitCast(rawPaths, to: NSArray.self) as! [String]
            for index in 0..<count {
                if watcher.paths.contains(changed[index]) || flags[index] & UInt32(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagRootChanged) != 0 {
                    watcher.continuation.yield(CaptureTrace.now())
                    break
                }
            }
        }, &context, [URL(fileURLWithPath: database).deletingLastPathComponent().path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.01, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagWatchRoot))
        if let events {
            FSEventStreamSetDispatchQueue(events, queue)
            if !FSEventStreamStart(events) { close() }
        }
    }

    func close() {
        if let events {
            FSEventStreamStop(events)
            FSEventStreamInvalidate(events)
            queue.sync {}
            FSEventStreamRelease(events)
        }
        events = nil
        continuation.finish()
    }
}
