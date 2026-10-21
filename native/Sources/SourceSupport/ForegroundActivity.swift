import Foundation

public final class ForegroundActivity: @unchecked Sendable {
    public static let shared = ForegroundActivity()
    private let lock = NSLock()
    private var active: Set<UUID> = []
    private var waiting: [CheckedContinuation<Void, Never>] = []
    private var interruptions: [UUID: @Sendable () -> Void] = [:]

    public func begin() -> UUID {
        let id = UUID()
        lock.lock()
        active.insert(id)
        let callbacks = Array(interruptions.values)
        lock.unlock()
        for callback in callbacks { callback() }
        return id
    }

    public func end(_ id: UUID) {
        lock.lock()
        active.remove(id)
        let ready = active.isEmpty ? waiting : []
        if active.isEmpty { waiting.removeAll() }
        lock.unlock()
        for continuation in ready { continuation.resume() }
    }

    public func waitUntilIdle() async {
        await withCheckedContinuation { continuation in
            lock.lock()
            if active.isEmpty {
                lock.unlock()
                continuation.resume()
            } else {
                waiting.append(continuation)
                lock.unlock()
            }
        }
    }

    public func interrupt(_ callback: @escaping @Sendable () -> Void) -> UUID {
        let id = UUID()
        lock.lock()
        interruptions[id] = callback
        let busy = !active.isEmpty
        lock.unlock()
        if busy { callback() }
        return id
    }

    public func removeInterruption(_ id: UUID) {
        lock.lock()
        interruptions.removeValue(forKey: id)
        lock.unlock()
    }
}
