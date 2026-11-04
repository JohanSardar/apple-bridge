import NativeProtocol
import Foundation
import SourceSupport
import FocusSource
import CalendarSource
import ContactsSource
import RemindersSource
import MacSource
import FilesSource
import MessagesSource
import MapsSource
import MediaDerivatives
import CryptoKit
import Contacts

extension AppleSession: NativeAdapter {
    public func effectsAllowed() async -> Bool { nativeAuthority && !ipcRunning && link == nil }
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

    public func runNative(descriptor: Int32) async throws {
        guard !ipcRunning, link == nil, !nativeAuthority else { throw SourceFailure.conflict }
        nativeAuthority = true
        defer { nativeAuthority = false; nativeFiles = nil }
        for (name, source) in sources.additional { additionalSources[name] = source }
        let root = home.appendingPathComponent("native-exchange")
        let files = try NativeFiles(root: root)
        defer { try? FileManager.default.removeItem(at: root) }
        nativeFiles = files
        try await NativeServer(adapter: self, files: files).run(descriptor: descriptor)
    }

    public func effect(_ operation: String, args: JSON, permit: JSON) async throws -> JSON {
        guard nativeAuthority, link == nil else { throw SourceFailure.conflict }
        var args = args
        if args["file_ref"] != .null {
            guard let nativeFiles else { throw SourceFailure.unavailable }
            let bytes = try await nativeFiles.read(args["file_ref"])
            var fields = args.object
            fields.removeValue(forKey: "file_ref")
            fields["data_base64"] = .string(bytes.base64EncodedString())
            args = .object(fields)
        }
        if operation == "media.derive" {
            guard let nativeFiles else { throw SourceFailure.unavailable }
            let bytes = try await nativeFiles.read(args["input"])
            let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
            guard digest == args["digest"].string else { throw SourceFailure.corrupt }
            let reference = try await nativeFiles.write(bytes, mime: args["input"]["mime"].string)
            let file = home.appendingPathComponent("native-exchange").appendingPathComponent(reference["path"].string)
            let result = try await MediaDerivatives.make(file, digest: digest)
            try await nativeFiles.release(reference)
            var encoded = try JSONDecoder().decode(JSON.self, from: JSONEncoder().encode(result)).object
            var derivatives: [String: JSON] = [:]
            for (kind, variant) in result.derivatives {
                let bytes = try Data(contentsOf: MediaDerivatives.url(variant))
                derivatives[kind] = try await nativeFiles.write(bytes, mime: variant.mime)
            }
            encoded["derivatives"] = .object(derivatives)
            return .object(encoded)
        }
        if let query = queryHandlers[operation] {
            guard args["file"] == .null else { throw SourceFailure.invalid }
            let output = try await query(args)
            if output.blobs.isEmpty { return output.result }
            guard let nativeFiles else { throw SourceFailure.unavailable }
            var refs: [JSON] = []
            for blob in output.blobs {
                refs.append(try await nativeBlob(handle: blob.handle, size: blob.size, digest: blob.digest, mime: "application/octet-stream", files: nativeFiles))
            }
            return .object(["result": output.result, "files": .array(refs)])
        }
        var fields = args.object
        fields["command_id"] = permit["command_id"]
        fields["request_hash"] = args["request_hash"] == .null ? permit["request_hash"] : args["request_hash"]
        fields["expires_at"] = .string(timestamp(Date(timeIntervalSince1970: Double(permit["expires_at"].unsigned))))
        var action = args["action"].object
        guard action["op"] == nil || action["op"] == .string(operation) else { throw SourceFailure.invalid }
        action["op"] = .string(operation)
        if args["action"] == .null { action["args"] = args }
        fields["action"] = .object(action)
        let command = JSON.object(fields)
        _ = try CommandOrigin(command)
        guard !command["principal"].string.isEmpty else { throw SourceFailure.invalid }
        if operation.hasPrefix("calendar.") {
            let prepared = try await calendar.prepare(command)
            try await calendar.validate(prepared)
            try nativeDeadline(permit)
            return try await nativeEvidence(prepared, result: try await calendar.dispatch(prepared))
        }
        if operation.hasPrefix("contacts.") {
            let prepared = try await contacts.prepare(command)
            try await contacts.validate(prepared)
            try nativeDeadline(permit)
            return try await nativeEvidence(prepared, result: try await contacts.dispatch(prepared))
        }
        if operation.hasPrefix("reminders.") {
            let prepared = try await source.prepare(command)
            try await source.validate(prepared)
            try nativeDeadline(permit)
            return try await nativeEvidence(prepared, result: try await source.dispatch(prepared))
        }
        guard let selected = additionalSources[captureRoute(operation)] else { throw SourceFailure.unsupported }
        let prepared = try await selected.prepare(command)
        if let messages = selected as? Messages { try await messages.validate(prepared) }
        try nativeDeadline(permit)
        return try await nativeEvidence(prepared, result: try await selected.dispatch(prepared))
    }

    private func nativeBlob(handle: String, size: UInt64, digest: String, mime: String, files: NativeFiles) async throws -> JSON {
        guard size <= 64 * 1024 * 1024, let reader = registeredBlobReaders.first(where: { handle.hasPrefix($0.key) })?.value else { throw SourceFailure.unavailable }
        var bytes = Data()
        while bytes.count < size {
            let chunk = try await reader.read(handle, bytes.count, min(49152, Int(size) - bytes.count))
            guard let part = Data(base64Encoded: chunk["bytes"].string), !part.isEmpty, part.count <= Int(size) - bytes.count else { throw SourceFailure.corrupt }
            bytes.append(part)
        }
        guard SHA256.hash(data: bytes).map({ String(format: "%02x", $0) }).joined() == digest else { throw SourceFailure.corrupt }
        return try await files.write(bytes, mime: mime)
    }

    private func nativeEvidence(_ prepared: Prepared, result: JSON) async throws -> JSON {
        var fields = result.object
        if !result["handle"].string.isEmpty, let nativeFiles {
            fields["file_ref"] = try await nativeBlob(handle: result["handle"].string, size: result["size"].unsigned, digest: result["digest"].string, mime: "application/octet-stream", files: nativeFiles)
        }
        fields["before"] = prepared.nativeState
        fields["fingerprint"] = .string(prepared.fingerprint)
        fields["candidate_ids"] = prepared.nativeState["candidate_ids"]
        return .object(fields)
    }

    private func nativeDeadline(_ permit: JSON) throws {
        try Task.checkCancellation()
        guard nativeAuthority, link == nil, permit["expires_at"].unsigned > UInt64(Date().timeIntervalSince1970) else { throw SourceFailure.expired }
    }
}
