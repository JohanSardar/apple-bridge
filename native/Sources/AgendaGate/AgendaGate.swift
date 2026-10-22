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

@globalActor
public actor AgendaActor {
    public static let shared = AgendaActor()
}

@AgendaActor
public final class AgendaGate {
    public let store = EKEventStore()
    public let trace: CaptureTrace
    public private(set) var notificationAt: UInt64 = 0
    public private(set) var debounceAt: UInt64 = 0
    public private(set) var calendarNotificationAt: UInt64 = 0
    public private(set) var calendarDebounceAt: UInt64 = 0
    private var observer: NSObjectProtocol?
    private var debounce: Task<Void, Never>?
    private var calendarDebounce: Task<Void, Never>?
    private var reminderFetches = 0
    private var reminderHintPending = false
    private var debounceWaiting: [CheckedContinuation<Void, Never>] = []
    private var calendarWaiting: [CheckedContinuation<Void, Never>] = []
    private var listeners: [UUID: (calendar: Bool, continuation: AsyncStream<Void>.Continuation)] = [:]

    public nonisolated init(trace: CaptureTrace = CaptureTrace()) {
        self.trace = trace
        Task { @AgendaActor [weak self] in
            guard let self else { return }
            observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: store, queue: nil) { @Sendable [weak self, trace] _ in
                let delivered = CaptureTrace.now()
                trace.record("notification", fields: ["delivered_ns": .integer(delivered)])
                Task(priority: .high) { @AgendaActor in self?.changed(delivered) }
            }
            var previous = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                let current = [EKEventStore.authorizationStatus(for: .reminder), EKEventStore.authorizationStatus(for: .event)]
                if current != previous { previous = current; self.reconcile() }
            }
        }

    }

    public func authorized(_ type: EKEntityType) -> Bool {
        EKEventStore.authorizationStatus(for: type) == .fullAccess
    }

    public func reconcile() {
        for listener in listeners.values { listener.continuation.yield(()) }
    }

    private func changed(_ delivered: UInt64) {
        guard debounce == nil else { return }
        reminderHintPending = true
        debounce = Task(priority: .high) { @AgendaActor in
            let activity = ForegroundActivity.shared.begin()
            defer {
                ForegroundActivity.shared.end(activity)
                debounce = nil
                let ready = debounceWaiting
                debounceWaiting.removeAll()
                for continuation in ready { continuation.resume() }
                resumeCalendar()
            }
            do { try await Task.sleep(for: .milliseconds(50)) } catch { return }
            notificationAt = max(notificationAt, delivered)
            debounceAt = CaptureTrace.now()
            trace.record("debounce", fields: ["notification_ns": .integer(notificationAt)])
            for listener in listeners.values where !listener.calendar { listener.continuation.yield(()) }
        }
    }

    private var reminderPending: Bool {
        debounce != nil || reminderFetches > 0 || (reminderHintPending && listeners.values.contains { !$0.calendar })
    }

    private func resumeCalendar() {
        guard !reminderPending else { return }
        let ready = calendarWaiting
        calendarWaiting.removeAll()
        for continuation in ready { continuation.resume() }
    }

    private func removeListener(_ id: UUID) {
        listeners.removeValue(forKey: id)
        resumeCalendar()
    }

    public func waitForReminderDebounce() async {
        if debounce != nil {
            await withCheckedContinuation { debounceWaiting.append($0) }
        }
    }

    public func waitForReminderRefetch() async {
        while reminderPending {
            await withCheckedContinuation { calendarWaiting.append($0) }
        }
    }

    public func calendarHint(_ delivered: UInt64) {
        trace.record("calendar_database_hint", fields: ["delivered_ns": .integer(delivered)])
        calendarChanged(delivered)
    }

    private func calendarChanged(_ delivered: UInt64) {
        guard calendarDebounce == nil else { return }
        calendarDebounce = Task(priority: .high) { @AgendaActor in
            let activity = ForegroundActivity.shared.begin()
            defer { ForegroundActivity.shared.end(activity) }
            do { try await Task.sleep(for: .milliseconds(50)) } catch { return }
            calendarNotificationAt = delivered
            calendarDebounceAt = CaptureTrace.now()
            calendarDebounce = nil
            trace.record("calendar_debounce", fields: ["notification_ns": .integer(delivered)])
            for listener in listeners.values where listener.calendar { listener.continuation.yield(()) }
        }
    }

    public func changes(calendar: Bool = false) -> AsyncStream<Void> {
        let id = UUID()
        return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            listeners[id] = (calendar, continuation)
            continuation.onTermination = { @Sendable [weak self] _ in
                Task { @AgendaActor in self?.removeListener(id) }
            }
        }
    }

    public func reminders() async throws -> [EKReminder] {
        guard authorized(.reminder) else { throw SourceFailure.permissionMissing }
        reminderFetches += 1
        defer {
            reminderFetches -= 1
            reminderHintPending = false
            resumeCalendar()
        }
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
