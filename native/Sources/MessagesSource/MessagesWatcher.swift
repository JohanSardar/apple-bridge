import Foundation
import AppKit
import Darwin

final class MessagesWatcher: @unchecked Sendable {
    let stream: AsyncStream<Bool>
    private let continuation: AsyncStream<Bool>.Continuation
    private let queue = DispatchQueue(label: "applebridge.messages.watch")
    private var sources: [String: (UInt64, any DispatchSourceFileSystemObject)] = [:]
    private var pending: DispatchWorkItem?
    private var timer: (any DispatchSourceTimer)?
    private var wake: (any NSObjectProtocol)?
    private let changeLock = NSLock()
    private var changed = false
    private var needsReconciliation = false
    private let path: String
    init(path: String) {
        self.path = path
        let pair = AsyncStream<Bool>.makeStream(bufferingPolicy: .bufferingNewest(1))
        stream = pair.stream
        continuation = pair.continuation
        queue.sync { rearm() }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 2, repeating: 2)
        timer.setEventHandler { [weak self] in self?.rearm(); self?.signal(false) }
        timer.resume()
        self.timer = timer
        continuation.yield(false)
        wake = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: nil) { [weak self] _ in
            self?.queue.async { [weak self] in self?.rearm(); self?.signal(true) }
        }
    }
    func requestReconciliation() {
        queue.async { [weak self] in self?.signal(true) }
    }
    private func signal(_ complete: Bool) {
        changeLock.withLock { changed = true; needsReconciliation = needsReconciliation || complete }
        continuation.yield(complete)
    }
    func consumeChanges() -> Bool {
        changeLock.withLock {
            let result = changed
            changed = false
            return result
        }
    }
    func consumeReconciliation() -> Bool {
        changeLock.withLock {
            let result = needsReconciliation
            needsReconciliation = false
            return result
        }
    }
    private func rearm() {
        let paths = [path, path + "-wal", path + "-shm", URL(fileURLWithPath: path).deletingLastPathComponent().path]
        for file in paths {
            let identity = ((try? FileManager.default.attributesOfItem(atPath: file)[.systemFileNumber]) as? NSNumber)?.uint64Value
            if sources[file]?.0 == identity { continue }
            sources.removeValue(forKey: file)?.1.cancel()
            guard let identity else { continue }
            let descriptor = Darwin.open(file, O_EVTONLY | O_CLOEXEC)
            guard descriptor >= 0 else { continue }
            let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .extend, .attrib, .delete, .rename, .revoke], queue: queue)
            source.setCancelHandler { Darwin.close(descriptor) }
            source.setEventHandler { [weak self] in
                guard let self else { return }
                rearm()
                pending?.cancel()
                let work = DispatchWorkItem { [weak self] in self?.signal(false) }
                pending = work
                queue.asyncAfter(deadline: .now() + .milliseconds(25), execute: work)
            }
            sources[file] = (identity, source)
            source.resume()
        }
    }
    func close() {
        queue.sync {
            pending?.cancel()
            timer?.cancel()
            timer = nil
            for (_, source) in sources.values { source.cancel() }
            sources.removeAll()
            continuation.finish()
        }
        if let wake { NSWorkspace.shared.notificationCenter.removeObserver(wake) }
        wake = nil
    }
}
