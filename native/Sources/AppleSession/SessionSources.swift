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
    struct Builtins: Sendable {
        let agenda: AgendaGate
        let reminders: RemindersSource
        let remindersStore: RemindersStoreEnrichment
        let calendar: CalendarSource
        let contacts: ContactsSource
        let usage: UsageSource

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
