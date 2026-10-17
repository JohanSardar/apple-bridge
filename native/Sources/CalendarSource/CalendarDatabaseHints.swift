import Foundation
import CoreServices
import Darwin
import SourceSupport

final class CalendarDatabaseHints: @unchecked Sendable {
    let changes: AsyncStream<UInt64>
    private let continuation: AsyncStream<UInt64>.Continuation
    private let queue = DispatchQueue(label: "applebridge.calendar.hints")
    private let paths: Set<String>
    private let directory: String
    private var files: [String: (UInt64, any DispatchSourceFileSystemObject)] = [:]
    private var events: FSEventStreamRef?

    init() {
        let database = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb").path
        paths = [database, database + "-wal"]
        directory = URL(fileURLWithPath: database).deletingLastPathComponent().path
        let pair = AsyncStream<UInt64>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream
        continuation = pair.continuation
        queue.sync { rearm() }
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        events = FSEventStreamCreate(nil, { _, info, count, rawPaths, flags, _ in
            guard let info else { return }
            let watcher = Unmanaged<CalendarDatabaseHints>.fromOpaque(info).takeUnretainedValue()
            let changed = unsafeBitCast(rawPaths, to: NSArray.self) as! [String]
            for index in 0..<count {
                if watcher.paths.contains(changed[index]) || flags[index] & UInt32(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagRootChanged) != 0 {
                    watcher.rearm()
                    watcher.continuation.yield(CaptureTrace.now())
                    break
                }
            }
        }, &context, [directory] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.01, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagWatchRoot))
        if let events {
            FSEventStreamSetDispatchQueue(events, queue)
            if !FSEventStreamStart(events) { close() }
        }
    }

    private func rearm() {
        for path in paths.union([directory]) {
            let identity = ((try? FileManager.default.attributesOfItem(atPath: path)[.systemFileNumber]) as? NSNumber)?.uint64Value
            if files[path]?.0 == identity { continue }
            files.removeValue(forKey: path)?.1.cancel()
            guard let identity else { continue }
            let descriptor = Darwin.open(path, O_EVTONLY | O_CLOEXEC)
            guard descriptor >= 0 else { continue }
            let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .extend, .delete, .rename, .revoke], queue: queue)
            source.setCancelHandler { Darwin.close(descriptor) }
            source.setEventHandler { [weak self] in
                guard let self else { return }
                rearm()
                continuation.yield(CaptureTrace.now())
            }
            files[path] = (identity, source)
            source.resume()
        }
    }

    func close() {
        if let events {
            FSEventStreamStop(events)
            FSEventStreamInvalidate(events)
            queue.sync {}
            FSEventStreamRelease(events)
        }
        queue.sync {
            for (_, source) in files.values { source.cancel() }
            files.removeAll()
        }
        events = nil
        continuation.finish()
    }
}
