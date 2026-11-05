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
import FocusSource
import SafariSource
import ApplicationServices

extension AppleSession: NativeAdapter {
    private func page(_ enumeration: SourceEnumeration, collection: String, handles: Bool) -> NativeFacts {
        var items = enumeration.rows.filter { $0["collection"].string == collection }
        if handles { items += enumeration.handles.keys.sorted().map { JSON.object(["kind": .string("blob_handle"), "digest": .string($0), "handle": .string(enumeration.handles[$0] ?? "")]) } }
        return NativeFacts(items: items, scope: .object(["collections": .array([.string(collection)]), "history_token": .string("")]), generation: enumeration.generation, complete: true)
    }

    private func nativeFacts(files paths: [String]) async throws -> NativeFacts {
        guard let source = additionalSources["files"] as? FilesSource else { throw SourceFailure.unavailable }
        return NativeFacts(items: source.ubiquity(paths), scope: .string("files"), generation: "native/1", complete: true)
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

    private func nativeFacts(single item: JSON, scope: String) -> NativeFacts {
        NativeFacts(items: [item], scope: .string(scope), generation: "native/1", complete: true)
    }

    public func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        if operation == "messages.media" { return try await nativeFacts(messagesMedia: args) }
        if operation == "focus.status" {
            guard args.object.isEmpty else { throw SourceFailure.unsupported }
            return nativeFacts(single: await FocusSource.frameworkStatus(), scope: "focus")
        }
        if operation == "calendar.candidates" {
            return NativeFacts(items: try await sources.calendar.candidates(args["query"]), scope: .string("calendar"), generation: "native/1", complete: true)
        }
        if operation == "calendar.account" {
            return nativeFacts(single: await sources.calendar.account(args["calendar"].string), scope: "calendar")
        }
        if operation == "mac.pasteboard" {
            guard args.object.isEmpty, let mac = additionalSources["mac"] as? MacSource else { throw SourceFailure.unsupported }
            return nativeFacts(single: try await mac.primitive("mac.pasteboard_policy", args: .object([:])), scope: "mac")
        }
        if operation == "accessibility.trusted" {
            guard args.object.isEmpty else { throw SourceFailure.unsupported }
            return nativeFacts(single: .object(["trusted": .bool(AXIsProcessTrusted())]), scope: "accessibility")
        }
        if operation == "safari.tabs" {
            guard args.object.isEmpty, let safari = additionalSources["safari"] as? SafariSource else { throw SourceFailure.unsupported }
            return NativeFacts(items: try await safari.liveTabs(), scope: .string("safari.tab"), generation: "native/1", complete: true)
        }
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
        let cache = sources.enumerations
        let calendar = sources.calendar, reminders = sources.reminders, contacts = sources.contacts
        switch operation {
        case "agenda.calendars", "agenda.events":
            let enumeration = try await cache.read("calendar") { try await calendar.enumerate() }
            return page(enumeration, collection: operation == "agenda.events" ? "calendar.event" : "calendar.calendar", handles: operation == "agenda.events")
        case "agenda.reminder_lists", "agenda.reminders":
            let enumeration = try await cache.read("reminders") { try await reminders.enumerate() }
            return page(enumeration, collection: operation == "agenda.reminders" ? "reminders.reminder" : "reminders.list", handles: false)
        case "contacts.containers", "contacts.groups", "contacts.contacts":
            let enumeration = try await cache.read("contacts") { try await contacts.enumerate() }
            let collection = ["contacts.containers": "contacts.container", "contacts.groups": "contacts.group", "contacts.contacts": "contacts.contact"][operation] ?? ""
            return page(enumeration, collection: collection, handles: operation == "contacts.contacts")
        default: throw SourceFailure.unsupported
        }
    }

    public func revalidate(_ operation: String, identities: [JSON]) async throws -> [String] {
        switch operation {
        case "agenda.calendars": return try await sources.calendar.revalidate(identities)
        case "agenda.reminder_lists", "agenda.reminders": return try await sources.reminders.revalidate(identities)
        case "contacts.containers", "contacts.groups", "contacts.contacts": return try await sources.contacts.revalidate(identities)
        default: throw SourceFailure.unsupported
        }
    }

    public func forwardChanges(to server: NativeServer) async {
        let cache = sources.enumerations, agenda = sources.agenda
        let photos = await sources.contacts.photosReady()
        let mac = additionalSources["mac"] as? MacSource
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
            group.addTask {
                for await _ in photos { await cache.invalidate("contacts"); await server.changed(source: "contacts", generation: "native/1") }
            }
            if let mac {
                group.addTask {
                    try? await mac.observe { live in if live { await server.changed(source: "mac", generation: "native/1") } }
                }
            }
        }
    }
}
