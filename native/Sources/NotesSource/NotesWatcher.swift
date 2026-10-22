import Foundation
import CoreServices
import Darwin
import SourceSupport

final class NotesWatcher: @unchecked Sendable {
    let changes: AsyncStream<Bool>
    private let continuation: AsyncStream<Bool>.Continuation
    private let queue = DispatchQueue(label: "applebridge.notes.events")
    private var events: FSEventStreamRef?
    private var pendingPaths = Set<String>()
    private let trace: CaptureTrace
    private let paths: [String]
    private var monitors: [String: (Int32, any DispatchSourceFileSystemObject)] = [:]
    private var timer: (any DispatchSourceTimer)?
    init(path: String, wal: String, trace: CaptureTrace) {
        self.trace = trace
        paths = [wal, String(wal.dropLast(4))]
        let pair = AsyncStream<Bool>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream; continuation = pair.continuation
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        events = FSEventStreamCreate(nil, { _, info, count, paths, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<NotesWatcher>.fromOpaque(info).takeUnretainedValue()
            watcher.trace.record("notes_fsevent_hint")
            watcher.refreshFiles()
            let names = paths.assumingMemoryBound(to: UnsafePointer<CChar>.self)
            watcher.request((0..<count).map { String(cString: names[$0]) })
        }, &context, [path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.1, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer))
        if let events { FSEventStreamSetDispatchQueue(events, queue); _ = FSEventStreamStart(events) }
        queue.sync { refreshFiles() }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 300, repeating: 300)
        timer.setEventHandler { [weak self] in self?.refreshFiles(); self?.request() }
        timer.resume(); self.timer = timer
        request()
    }
    func request(_ paths: [String] = []) {
        queue.async { [weak self] in
            guard let self else { return }
            self.pendingPaths.formUnion(paths); self.continuation.yield(true)
        }
    }
    func takePaths() -> Set<String> {
        queue.sync { let result = pendingPaths; pendingPaths = []; return result }
    }
    private func refreshFiles() {
        for path in paths {
            if let (descriptor, monitor) = monitors[path] {
                var opened = stat(), current = stat()
                if fstat(descriptor, &opened) == 0, stat(path, &current) == 0, opened.st_dev == current.st_dev, opened.st_ino == current.st_ino { continue }
                monitor.cancel(); monitors.removeValue(forKey: path)
            }
            let descriptor = Darwin.open(path, O_EVTONLY | O_CLOEXEC)
            guard descriptor >= 0 else { continue }
            let monitor = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .extend, .rename, .delete], queue: queue)
            monitor.setEventHandler { [weak self] in
                guard let self else { return }
                self.trace.record(path.hasSuffix("-wal") ? "notes_wal_changed" : "notes_database_changed")
                self.refreshFiles(); self.request()
            }
            monitor.setCancelHandler { Darwin.close(descriptor) }
            monitors[path] = (descriptor, monitor)
            monitor.resume()
        }
    }
    func close() {
        if let events { FSEventStreamStop(events); FSEventStreamInvalidate(events); FSEventStreamRelease(events) }
        events = nil
        queue.sync {
            timer?.cancel(); timer = nil
            for (_, monitor) in monitors.values { monitor.cancel() }
            monitors = [:]
        }
        continuation.finish()
    }
}
