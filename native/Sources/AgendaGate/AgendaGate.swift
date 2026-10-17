import Foundation
@preconcurrency import EventKit
import SourceSupport

private struct ReminderFetch: @unchecked Sendable {
    let values: [EKReminder]?
}

private final class ReminderCompletion: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<ReminderFetch, any Error>?

    init(_ continuation: CheckedContinuation<ReminderFetch, any Error>) { self.continuation = continuation }

    func finish(_ result: Result<ReminderFetch, any Error>) {
        lock.lock()
        let pending = continuation
        continuation = nil
        lock.unlock()
        pending?.resume(with: result)
    }
}

@MainActor
public final class AgendaGate {
    public let store = EKEventStore()
    public let trace: CaptureTrace
    public private(set) var notificationAt: UInt64 = 0
    public private(set) var debounceAt: UInt64 = 0
    private var observer: NSObjectProtocol?
    private var debounce: Task<Void, Never>?
    private var listeners: [UUID: AsyncStream<Void>.Continuation] = [:]

    public init(trace: CaptureTrace = CaptureTrace()) {
        self.trace = trace
        Task { @MainActor [weak self] in
            var previous = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self else { return }
                let current = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
                if current != previous { previous = current; self.reconcile() }
            }
        }
        observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: store, queue: .main) { @Sendable [weak self, trace] _ in
            let delivered = CaptureTrace.now()
            trace.record("notification")
            Task { @MainActor in self?.changed(delivered) }
        }
    }

    public func authorized(_ type: EKEntityType) -> Bool {
        EKEventStore.authorizationStatus(for: type) == .fullAccess
    }

    public func reconcile() {
        for continuation in listeners.values { continuation.yield(()) }
    }

    private func changed(_ delivered: UInt64) {
        debounce?.cancel()
        debounce = Task { @MainActor in
            do { try await Task.sleep(for: .milliseconds(50)) } catch { return }
            notificationAt = delivered
            debounceAt = CaptureTrace.now()
            trace.record("debounce", fields: ["notification_ns": .integer(notificationAt)])
            for continuation in listeners.values { continuation.yield(()) }
        }
    }

    public func changes() -> AsyncStream<Void> {
        let id = UUID()
        return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            listeners[id] = continuation
            continuation.onTermination = { @Sendable [weak self] _ in
                Task { @MainActor in self?.listeners.removeValue(forKey: id) }
            }
        }
    }

    public func reminders() async throws -> [EKReminder] {
        guard authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let trace = self.trace
        let predicate = store.predicateForReminders(in: nil)
        let fetched: ReminderFetch = try await withCheckedThrowingContinuation { continuation in
            let completion = ReminderCompletion(continuation)
            DispatchQueue.global().asyncAfter(deadline: .now() + 3) { @Sendable in
                trace.record("fetch_timer_fired")
                completion.finish(.failure(SourceFailure.unavailable))
            }
            store.fetchReminders(matching: predicate) { result in completion.finish(.success(ReminderFetch(values: result))) }
        }
        try Task.checkCancellation()
        guard authorized(.reminder) else { throw SourceFailure.permissionMissing }
        guard let reminders = fetched.values else { throw SourceFailure.unavailable }
        return reminders
    }
}
