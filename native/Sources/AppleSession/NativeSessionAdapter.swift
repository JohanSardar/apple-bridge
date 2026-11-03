import NativeProtocol
import Foundation
import SourceSupport
import FocusSource
import CalendarSource
import ContactsSource
import RemindersSource
import MacSource
import MessagesSource
import MapsSource
import MediaDerivatives
import CryptoKit

extension AppleSession: NativeAdapter {
    public func effectsAllowed() async -> Bool { nativeAuthority && !ipcRunning && wire == nil }
    public func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        guard args.object.isEmpty || operation == "contacts.history" || operation == "ui.target" else { throw SourceFailure.unsupported }
        let sink = NativeFactSink()
        let builtins: SessionSources.Builtins
        if let nativeSources { builtins = nativeSources }
        else {
            let root = home.appendingPathComponent("native-facts")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            builtins = try await SessionSources.Builtins(home: root, labEnabled: false, trace: CaptureTrace())
            nativeSources = builtins
        }
        let calendar = builtins.calendar, source = builtins.reminders, contacts = builtins.contacts
        switch operation {
        case "agenda.calendars", "agenda.events":
            guard (await calendar.open()).state == "ready" else { throw SourceFailure.permissionMissing }
            try await calendar.nativeFacts(into: sink)
            return await sink.result(collection: operation == "agenda.events" ? "calendar.event" : "calendar.calendar")
        case "agenda.reminder_lists", "agenda.reminders":
            guard (await source.open()).state == "ready" else { throw SourceFailure.permissionMissing }
            try await source.nativeFacts(into: sink)
            return await sink.result(collection: operation == "agenda.reminders" ? "reminders.reminder" : "reminders.list")
        case "contacts.history":
            guard Set(args.object.keys) == ["history_token"] else { throw SourceFailure.invalid }
            let history = try await contacts.nativeHistory(args["history_token"].string)
            let changes = history["changed_contact_ids"].array.map { JSON.object(["collection": .string("contacts.contact"), "native_key": $0]) }
                + history["changed_group_ids"].array.map { JSON.object(["collection": .string("contacts.group"), "native_key": $0]) }
            return NativeFacts(items: changes, scope: .object(["collections": .array([.string("contacts.contact"), .string("contacts.group")]), "history_token": history["history_token"]]), generation: "native/1", complete: true)
        case "contacts.containers", "contacts.groups", "contacts.contacts":
            try await contacts.nativeFacts(into: sink)
            let collection = ["contacts.containers": "contacts.container", "contacts.groups": "contacts.group", "contacts.contacts": "contacts.contact"][operation]
            return await sink.result(collection: collection)
        case "location.status":
            let ready = await LocationStatus().open()
            return NativeFacts(items: [.object(["state": .string(ready.state), "reason": .string(ready.reason)])], scope: .string("location.status"), generation: "native/1", complete: true)
        case "focus.status":
            return NativeFacts(items: [await FocusSource.status()], scope: .string("focus.status"), generation: "native/1", complete: true)
        case "permissions.status":
            let states = await nativePermissionStatus()
            return NativeFacts(items: [states], scope: .string("permissions.status"), generation: "native/1", complete: true)
        case "mac.state":
            let mac = await MacSource()
            try await mac.nativeFacts(into: sink)
            return await sink.result()
        case "ui.target":
            return NativeFacts(items: [try await MessagesAccessibility.nativeTarget(args)], scope: .string("ui.target"), generation: "native/1", complete: true)
        case "ui.actions":
            guard let messages = additionalSources["messages"] else { throw SourceFailure.unavailable }
            let ready = await messages.open()
            return NativeFacts(items: ready.capabilities.map(JSON.string), scope: .string("ui.actions"), generation: "native/1", complete: true)
        default: throw SourceFailure.unsupported
        }
    }

    public func runNative(descriptor: Int32) async throws {
        guard !ipcRunning, wire == nil, !nativeAuthority else { throw SourceFailure.conflict }
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
        guard nativeAuthority, wire == nil else { throw SourceFailure.conflict }
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
        guard nativeAuthority, wire == nil, permit["expires_at"].unsigned > UInt64(Date().timeIntervalSince1970) else { throw SourceFailure.expired }
    }

    @MainActor
    private func nativePermissionStatus() async -> JSON {
        let session = PermissionSession()
        session.refresh()
        var states = session.report.snapshot()
        states.merge(await NewPermissions.status()) { _, new in new }
        return .object(states.mapValues(JSON.string))
    }
}
