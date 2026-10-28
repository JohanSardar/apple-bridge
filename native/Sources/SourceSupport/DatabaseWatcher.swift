import Foundation
import Darwin

public final class DatabaseWatcher: @unchecked Sendable {
    public let changes: AsyncStream<Void>
    private let continuation: AsyncStream<Void>.Continuation
    private let queue = DispatchQueue(label: "applebridge.database.watch")
    private let queueKey = DispatchSpecificKey<Bool>()
    private let paths: [String]
    private let coalescing: DispatchTimeInterval
    private var monitors: [String: (Int32, any DispatchSourceFileSystemObject)] = [:]
    private var pending: DispatchWorkItem?
    private var timer: (any DispatchSourceTimer)?
    private var closed = false

    public init(path: String, coalescing: DispatchTimeInterval = .milliseconds(25), pollInterval: DispatchTimeInterval = .seconds(30)) {
        paths = [path, path + "-wal", path + "-shm", URL(fileURLWithPath: path).deletingLastPathComponent().path]
        self.coalescing = coalescing
        let pair = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream
        continuation = pair.continuation
        queue.setSpecific(key: queueKey, value: true)
        queue.sync { rearm() }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + pollInterval, repeating: pollInterval)
        timer.setEventHandler { [weak self] in self?.rearm(); self?.signal() }
        self.timer = timer
        timer.resume()
    }

    private func signal() {
        guard !closed, pending == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.closed else { return }
            self.pending = nil
            self.continuation.yield(())
        }
        pending = work
        queue.asyncAfter(deadline: .now() + coalescing, execute: work)
    }

    private func rearm() {
        guard !closed else { return }
        for path in paths {
            if let (descriptor, monitor) = monitors[path] {
                var opened = stat(), current = stat()
                if fstat(descriptor, &opened) == 0, stat(path, &current) == 0, opened.st_dev == current.st_dev, opened.st_ino == current.st_ino { continue }
                monitor.cancel()
                monitors.removeValue(forKey: path)
            }
            let descriptor = Darwin.open(path, O_EVTONLY | O_CLOEXEC)
            guard descriptor >= 0 else { continue }
            let monitor = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .extend, .attrib, .delete, .rename, .revoke], queue: queue)
            monitor.setCancelHandler { Darwin.close(descriptor) }
            monitor.setEventHandler { [weak self] in self?.rearm(); self?.signal() }
            monitors[path] = (descriptor, monitor)
            monitor.resume()
        }
    }

    public func close() {
        if DispatchQueue.getSpecific(key: queueKey) == true { stop() }
        else { queue.sync { stop() } }
    }

    private func stop() {
        guard !closed else { return }
        closed = true
        pending?.cancel()
        pending = nil
        timer?.cancel()
        timer = nil
        for (_, monitor) in monitors.values { monitor.cancel() }
        monitors.removeAll()
        continuation.finish()
    }

    deinit { close() }
}
