import Foundation
import SourceSupport
import AgendaGate
import RemindersSource
import RemindersStore
import CalendarSource
import ContactsSource
import UsageSource

@MainActor
internal enum SessionSources {
    static func rustCaptureSources(home: URL) -> Set<String> {
        guard let data = try? Data(contentsOf: home.appendingPathComponent("session/rust-capture.json")),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let sources = object["sources"] as? [String: Any] else { return [] }
        return Set(sources.keys)
    }
    struct Builtins: Sendable {
        let agenda: AgendaGate
        let reminders: RemindersSource
        let remindersStore: RemindersStoreEnrichment
        let calendar: CalendarSource
        let contacts: ContactsSource
        let usage: UsageSource
        let enumerations = EnumerationCache()

        var additional: [String: any Source] {
            [contacts.name: contacts, usage.name: usage]
        }

        @MainActor
        init(home: URL, labEnabled: Bool, trace: CaptureTrace) throws {
            agenda = AgendaGate(trace: trace)

            usage = UsageSource()

            contacts = try ContactsSource(home: home, commandsEnabled: true, trace: trace)

            reminders = try RemindersSource(gate: agenda, home: home, commandsEnabled: true)
            remindersStore = RemindersStoreEnrichment(trace: labEnabled ? trace : nil)

            calendar = try CalendarSource(gate: agenda, home: home, commandsEnabled: true, profilingEnabled: labEnabled)
        }
    }
}

actor EnumerationCache {
    private var entries: [String: (at: Date, task: Task<SourceEnumeration, Error>)] = [:]

    func read(_ source: String, ttl: TimeInterval = 5, _ make: @escaping @Sendable () async throws -> SourceEnumeration) async throws -> SourceEnumeration {
        if let entry = entries[source], Date().timeIntervalSince(entry.at) < ttl { return try await entry.task.value }
        let task = Task { try await make() }
        entries[source] = (Date(), task)
        do { return try await task.value }
        catch { entries[source] = nil; throw error }
    }

    func invalidate(_ source: String) { entries[source] = nil }
}
