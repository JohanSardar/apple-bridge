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
    private var observer: NSObjectProtocol?
    private var debounce: Task<Void, Never>?
    private var listeners: [UUID: AsyncStream<Void>.Continuation] = [:]

    public init() {
        Task { @MainActor [weak self] in
            var previous = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self else { return }
                let current = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
                if current != previous { previous = current; self.reconcile() }
            }
        }
        observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: store, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.changed() }
        }
    }

    public func authorized(_ type: EKEntityType) -> Bool {
        EKEventStore.authorizationStatus(for: type) == .fullAccess
    }

    public func reconcile() {
        for continuation in listeners.values { continuation.yield(()) }
    }

    private func changed() {
        debounce?.cancel()
        debounce = Task { @MainActor in
            do { try await Task.sleep(for: .milliseconds(50)) } catch { return }
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
        let predicate = store.predicateForReminders(in: nil)
        let fetched: ReminderFetch = try await withCheckedThrowingContinuation { continuation in
            let completion = ReminderCompletion(continuation)
            DispatchQueue.global().asyncAfter(deadline: .now() + 3) {
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
