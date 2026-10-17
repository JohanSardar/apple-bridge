import Foundation
@preconcurrency import EventKit
import SourceSupport

private struct ReminderFetch: @unchecked Sendable {
    let values: [EKReminder]?
}

@MainActor
public final class AgendaGate {
    public let store = EKEventStore()
    private var observer: NSObjectProtocol?
    private var debounce: Task<Void, Never>?
    private var listeners: [UUID: AsyncStream<Void>.Continuation] = [:]

    public init() {
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
        return AsyncStream { continuation in
            listeners[id] = continuation
            continuation.onTermination = { @Sendable [weak self] _ in
                Task { @MainActor in self?.listeners.removeValue(forKey: id) }
            }
        }
    }

    public func reminders() async throws -> [EKReminder] {
        guard authorized(.reminder) else { throw SourceFailure.permissionMissing }
        let predicate = store.predicateForReminders(in: nil)
        let fetched: ReminderFetch = await withCheckedContinuation { continuation in
            store.fetchReminders(matching: predicate) { result in continuation.resume(returning: ReminderFetch(values: result)) }
        }
        guard authorized(.reminder) else { throw SourceFailure.permissionMissing }
        guard let reminders = fetched.values else { throw SourceFailure.unavailable }
        return reminders
    }
}
