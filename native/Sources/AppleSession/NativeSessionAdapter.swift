import NativeProtocol
import Foundation
import SourceSupport
import CalendarSource
import ContactsSource
import RemindersSource
import MacSource
import FilesSource
import MessagesSource
import MediaDerivatives
import Contacts

extension AppleSession: NativeAdapter {
    private func agendaSources() async throws -> SessionSources.Builtins {
        if let nativeSources { return nativeSources }
        if let nativeSourcesTask { return try await nativeSourcesTask.value }
        let home = self.home
        let task = Task { @MainActor () throws -> SessionSources.Builtins in
            let root = home.appendingPathComponent("native-facts")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            return try SessionSources.Builtins(home: root, labEnabled: false, trace: CaptureTrace())
        }
        nativeSourcesTask = task
        do {
            let builtins = try await task.value
            nativeSources = builtins
            return builtins
        } catch {
            nativeSourcesTask = nil
            throw error
        }
    }

    private func page(_ enumeration: SourceEnumeration, collection: String, handles: Bool) -> NativeFacts {
        var items = enumeration.rows.filter { $0["collection"].string == collection }
        if handles { items += enumeration.handles.keys.sorted().map { JSON.object(["kind": .string("blob_handle"), "digest": .string($0), "handle": .string(enumeration.handles[$0] ?? "")]) } }
        return NativeFacts(items: items, scope: .object(["collections": .array([.string(collection)]), "history_token": .string("")]), generation: enumeration.generation, complete: true)
    }

    private func nativeFacts(files paths: [String]) async throws -> NativeFacts {
        guard let source = additionalSources["files"] as? FilesSource else { throw SourceFailure.unavailable }
        return NativeFacts(items: try await source.ubiquity(paths), scope: .string("files"), generation: "native/1", complete: true)
    }

    private func nativeFacts(macState source: MacSource) async throws -> NativeFacts {
        let state = try await MainActor.run { try source.state() }
        let rows = state.sorted { $0.key < $1.key }.map { key, value -> JSON in
            let parts = key.split(separator: "/", maxSplits: 1).map(String.init)
            return .object(["kind": .string("upsert"), "collection": .string(parts[0]), "native_key": .string(parts[1]), "data": value])
        }
        return NativeFacts(items: rows, scope: .string("mac"), generation: "native/1", complete: true)
    }

    private func nativeFacts(messagesMedia args: JSON) async throws -> NativeFacts {
        guard let messages = additionalSources["messages"] as? Messages else { throw SourceFailure.unavailable }
        guard let fields = try JSONSerialization.jsonObject(with: JSONEncoder().encode(args)) as? [String: Any] else { throw SourceFailure.invalid }
        let answer: [String: Any]
        do { answer = try await MessagesMediaFact.answer(fields, allowedRoot: messages.attachmentsRoot) } catch { throw SourceFailure.invalid }
        let item = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: answer, options: [.sortedKeys]))
        return NativeFacts(items: [item], scope: .string("messages.media"), generation: "native/1", complete: true)
    }

    public func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        if operation == "messages.media" { return try await nativeFacts(messagesMedia: args) }
        if operation == "files.ubiquity" {
            guard Set(args.object.keys) == ["paths"], case .array(let paths) = args["paths"], paths.allSatisfy({ if case .string = $0 { return true } else { return false } }) else { throw SourceFailure.invalid }
            return try await nativeFacts(files: paths.map(\.string))
        }
        if operation == "mac.state" {
            guard args.object.isEmpty else { throw SourceFailure.unsupported }
            guard let source = additionalSources["mac"] as? MacSource else { throw SourceFailure.unavailable }
            return try await nativeFacts(macState: source)
        }
        guard args.object.isEmpty else { throw SourceFailure.unsupported }
        let builtins = try await agendaSources()
        let cache = builtins.enumerations
        let calendar = builtins.calendar, reminders = builtins.reminders, contacts = builtins.contacts
        switch operation {
        case "agenda.calendars", "agenda.events":
            let rustStore = await SessionSources.rustStore(home: home, source: "calendar")
            let enumeration = try await cache.read("calendar") { try await calendar.enumerate(storeInRust: rustStore) }
            return page(enumeration, collection: operation == "agenda.events" ? "calendar.event" : "calendar.calendar", handles: operation == "agenda.events")
        case "agenda.reminder_lists", "agenda.reminders":
            let enrichment = builtins.remindersStore
            let rustStore = await SessionSources.rustStore(home: home, source: "reminders")
            let enumeration = try await cache.read("reminders") {
                let raw = try await reminders.enumerate()
                if rustStore { return raw }
                return SourceEnumeration(generation: raw.generation, rows: await enrichment.refresh(raw.rows, generation: raw.generation), handles: raw.handles)
            }
            return page(enumeration, collection: operation == "agenda.reminders" ? "reminders.reminder" : "reminders.list", handles: false)
        case "contacts.containers", "contacts.groups", "contacts.contacts":
            let enumeration = try await cache.read("contacts") { try await contacts.enumerate() }
            let collection = ["contacts.containers": "contacts.container", "contacts.groups": "contacts.group", "contacts.contacts": "contacts.contact"][operation] ?? ""
            return page(enumeration, collection: collection, handles: operation == "contacts.contacts")
        default: throw SourceFailure.unsupported
        }
    }

    public func revalidate(_ operation: String, identities: [JSON]) async throws -> [String] {
        let builtins = try await agendaSources()
        switch operation {
        case "agenda.calendars", "agenda.events": return try await builtins.calendar.revalidate(identities)
        case "agenda.reminder_lists", "agenda.reminders": return try await builtins.reminders.revalidate(identities)
        case "contacts.containers", "contacts.groups", "contacts.contacts": return try await builtins.contacts.revalidate(identities)
        default: throw SourceFailure.unsupported
        }
    }

    public func forwardChanges(to server: NativeServer) async {
        guard let builtins = try? await agendaSources() else { return }
        let cache = builtins.enumerations, agenda = builtins.agenda
        let mac = CaptureOwnership(home: home).rust.contains("mac") ? additionalSources["mac"] as? MacSource : nil
        await withTaskGroup(of: Void.self) { group in
            group.addTask {
                for await _ in await agenda.changes(calendar: true) { await cache.invalidate("calendar"); await server.changed(source: "calendar", generation: "native/1") }
            }
            group.addTask {
                for await _ in await agenda.changes() {
                    await cache.invalidate("reminders")
                    await server.changed(source: "reminders", generation: "native/1")
                    await cache.invalidate("calendar")
                    await server.changed(source: "calendar", generation: "native/1")
                }
            }
            group.addTask {
                for await _ in NotificationCenter.default.notifications(named: .CNContactStoreDidChange) { await cache.invalidate("contacts"); await server.changed(source: "contacts", generation: "native/1") }
            }
            if let mac {
                group.addTask {
                    try? await mac.observe { live in if live { await server.changed(source: "mac", generation: "native/1") } }
                }
            }
        }
    }
}
