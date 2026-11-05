import Foundation
import SourceSupport
import AgendaGate
import RemindersSource
import CalendarSource
import ContactsSource

@MainActor
internal enum SessionSources {
    nonisolated static let rustDecidesReadiness: Set<String> = ["mail", "shortcuts", "files", "notes", "messages", "mac"]

    struct Builtins: Sendable {
        let agenda: AgendaGate
        let reminders: RemindersSource
        let calendar: CalendarSource
        let contacts: ContactsSource
        let enumerations = EnumerationCache()

        var additional: [String: any Source] { [contacts.name: contacts] }

        @MainActor
        init(home: URL) throws {
            agenda = AgendaGate()
            contacts = try ContactsSource(home: home, commandsEnabled: true)
            reminders = RemindersSource(gate: agenda, commandsEnabled: true)
            calendar = try CalendarSource(gate: agenda, home: home, commandsEnabled: true)
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
