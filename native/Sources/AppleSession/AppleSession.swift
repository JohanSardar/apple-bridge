import NativeProtocol
import PDFSource
import FilesSource
import MailSource
import Foundation
import CryptoKit
import Darwin
import SourceSupport
import AgendaGate
import RemindersSource
import NotesSource
import ContactsSource
import CalendarSource
import MessagesSource

public actor AppleSession {
    private struct Ticket {
        let command: JSON
        let prepared: Prepared
        let expiry: ContinuousClock.Instant
        let fence: UInt64
    }
    private var running = false
    var link: NativeSessionLink?
    private var facts: NativeServer?
    private var connected: Bool { link != nil }
    private var fence: UInt64 = 0
    private var permissionStates: [String: String] = [:]
    private var commandActive = false
    var additionalSources: [String: any Source] = [:]
    private var queryAudio: [String: (Data, Date)] = [:]
    var queryHandlers: [String: @Sendable (JSON) async throws -> QueryOutput] = [:]
    struct RegisteredBlobReader: Sendable {
        let source: String
        let read: @Sendable (String, Int, Int) async throws -> JSON
    }
    var registeredBlobReaders: [String: RegisteredBlobReader] = [:]
    public func registerQuery(_ name: String, handler: @escaping @Sendable (JSON) async throws -> JSON) throws {
        try registerQueryOutput(name) { args in QueryOutput(result: try await handler(args)) }
    }
    public func registerQueryOutput(_ name: String, handler: @escaping @Sendable (JSON) async throws -> QueryOutput) throws {
        guard link == nil, queryHandlers[name] == nil else { throw SourceFailure.conflict }
        queryHandlers[name] = handler
    }
    public func registerBlobReader(_ prefix: String, source: String, reader: @escaping @Sendable (String, Int, Int) async throws -> JSON) throws {
        guard link == nil, !prefix.isEmpty, !source.isEmpty, !registeredBlobReaders.keys.contains(where: { $0.hasPrefix(prefix) || prefix.hasPrefix($0) }) else { throw SourceFailure.conflict }
        registeredBlobReaders[prefix] = RegisteredBlobReader(source: source, read: reader)
    }
    private var queryTasks: [String: Task<Void, Never>] = [:]
    private var frameTasks: [UUID: Task<Void, Never>] = [:]
    let sources: SessionSources.Builtins
    var contacts: ContactsSource { sources.contacts }
    var source: RemindersSource { sources.reminders }
    var calendar: CalendarSource { sources.calendar }
    private let socket: String
    private let secretFile: String
    private let gatewayPID: Int32
    let home: URL

    @MainActor
    public init(socket: String, secretFile: String, home: URL, gatewayPID: Int32, commandsEnabled: Bool) throws {
        self.socket = socket; self.secretFile = secretFile; self.gatewayPID = gatewayPID
        self.home = home
        sources = try SessionSources.Builtins(home: home)
        let pidFile = home.appendingPathComponent("helper.pid")
        try Data(String(getpid()).utf8).write(to: pidFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pidFile.path)
    }

    public func register(_ source: any Source) throws {
        guard link == nil, source.name != "reminders", additionalSources[source.name] == nil else { throw SourceFailure.conflict }
        additionalSources[source.name] = source
    }

    private func disconnect() {
        link?.close(); link = nil
        permissionStates = [:]
        for task in frameTasks.values { task.cancel() }
        frameTasks = [:]
        for task in queryTasks.values { task.cancel() }; queryTasks = [:]
    }

    public func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        guard let link else { throw SourceFailure.unavailable }
        do {
            try link.send(NativeSessionFrame.sourceState(source: source, state: state, coverage: coverage, capabilities: capabilities, reason: reason))
        }
    }

    private var frameworkSources: [any Source] {
        [source, calendar] + additionalSources.values.filter { !SessionSources.rustDecidesReadiness.contains($0.name) }
    }

    private func publishReadiness(force: Bool = false) async throws {
        for source in frameworkSources {
            let readiness = await source.open()
            if force || readiness.state != permissionStates[source.name] {
                permissionStates[source.name] = readiness.state
                try await sourceState(source: source.name, state: readiness.state, coverage: readiness.coverage, capabilities: readiness.capabilities, reason: readiness.reason)
            }
        }
    }




    private func launchFrame(_ frame: JSON) {
        let id = UUID()
        frameTasks[id] = Task {
            switch frame["t"].string {
            case "Prepare": await nativePrepare(frame)
            case "Dispatch": await nativeDispatch(frame)
            case "Resolve": await nativeResolve(frame)
            case "BlobRead": await readBlob(frame)
            case "Primitive": await nativePrimitive(frame)
            default: break
            }
            frameTasks.removeValue(forKey: id)
        }
    }

    private func transmit(_ frame: JSON) {
        try? link?.send(frame)
    }

    private func query(_ frame: JSON) async {
        guard !Task.isCancelled else { return }
        let id = frame["id"]
        if let handler = queryHandlers[frame["op"].string] {
            do {
                let output = try await handler(frame["args"])
                var result = output.result
                var blobs: [JSON] = output.blobs.map(\.wireReference)
                if frame["op"].string == "speech.synthesize", let reference = try cacheQueryAudio(&result) { blobs = [reference] }
                guard !Task.isCancelled else { return }
                let encoded = try JSONEncoder().encode(result)
                guard encoded.count <= 1 << 20 else { throw SourceFailure.invalid }
                guard output.blobs.count <= 16, output.blobs.allSatisfy({ $0.size <= 64 << 20 && !$0.handle.isEmpty }) else { throw SourceFailure.invalid }

                blobs = frame["op"].string == "files.read" ? [.object(["digest": result["digest"], "size": result["size"], "handle": .string("files-blob:" + result["digest"].string)])] : blobs
                transmit(.object(["t": .string("QueryResult"), "id": id, "result_json": result, "blobs": .array(blobs)]))
            } catch {
                let code = error is CancellationError ? "deadline" : (error as? SourceFailure == .invalid ? "invalid" : (error as? SourceFailure == .unsupported ? "unsupported" : "unavailable"))
                transmit(.object(["t": .string("QueryError"), "id": id, "code": .string(code)]))
            }
            return
        }
        guard frame["op"].string == "helper.version" else {
            transmit(.object(["t": .string("QueryError"), "id": id, "code": .string("unsupported")]))
            return
        }
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development"
        let result = JSON.object(["version": .string(version), "helper_build": .string("m1-session"), "protocol": .string("ipc/1")])
        guard !Task.isCancelled else { return }
        transmit(.object(["t": .string("QueryResult"), "id": id, "result_json": result, "blobs": .array([])]))
    }

    func cacheQueryAudio(_ result: inout JSON) throws -> JSON? {
        guard let data = Data(base64Encoded: result["data_base64"].string), !data.isEmpty else { return nil }
        queryAudio = queryAudio.filter { Date().timeIntervalSince($0.value.1) < 60 }
        guard queryAudio.count < 8 else { throw SourceFailure.unavailable }
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        let handle = "query-audio:" + UUID().uuidString
        queryAudio[handle] = (data, Date())
        result = .object(["media_type": .string("audio/wav")])
        return .object(["digest": .string(digest), "size": .number(Double(data.count)), "handle": .string(handle)])
    }

    private func readBlob(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let nativeLink = link else { return }
        func current() -> Bool { link === nativeLink }
        let registered = registeredBlobReaders.first(where: { frame["handle"].string.hasPrefix($0.key) })?.value
        var contactsVariant = false
        if registered == nil, frame["handle"].string.hasPrefix("media:") { contactsVariant = await contacts.serves(frame["handle"].string) }
        do {
            guard !frame["handle"].string.isEmpty, frame["offset"].number >= 0, frame["offset"].number < Double(Int64.max), frame["length"].number > 0, frame["length"].number <= 131072, frame["length"].number.rounded(.down) == frame["length"].number, frame["offset"].number.rounded(.down) == frame["offset"].number else { throw SourceFailure.invalid }
            let response: JSON
            if let registered { response = try await registered.read(frame["handle"].string, Int(frame["offset"].number), Int(frame["length"].number)) }
            else if frame["handle"].string.hasPrefix("query-audio:") {
                guard let cached = queryAudio[frame["handle"].string], Date().timeIntervalSince(cached.1) < 60 else { throw SourceFailure.unavailable }
                let offset = Int(frame["offset"].number)
                guard offset <= cached.0.count else { throw SourceFailure.invalid }
                let end = min(cached.0.count, offset + Int(frame["length"].number))
                let digest = SHA256.hash(data: cached.0).map { String(format: "%02x", $0) }.joined()
                response = .object(["t": .string("BlobChunk"), "handle": frame["handle"], "offset": frame["offset"], "bytes": .string(cached.0.subdata(in: offset..<end).base64EncodedString()), "digest": .string(digest), "eof": .bool(end == cached.0.count)])
            }
            else if frame["handle"].string.hasPrefix("contacts-image:") || contactsVariant { response = try await contacts.readBlob(frame["handle"].string, offset: Int(frame["offset"].number), length: Int(frame["length"].number)) }
            else if frame["handle"].string.hasPrefix("pdf:") {
                guard let pdf = additionalSources["pdf"] as? PDFSource else { throw SourceFailure.unavailable }
                response = try await pdf.readBlob(frame["handle"].string, offset: Int64(frame["offset"].number), length: Int(frame["length"].number))
            }
            else if frame["handle"].string.hasPrefix("files-blob:") {
                guard let files = additionalSources["files"] as? FilesSource else { throw SourceFailure.unavailable }
                response = try await files.readBlob(frame["handle"].string, offset: Int64(frame["offset"].number), length: Int(frame["length"].number))
            }
            else { throw SourceFailure.unavailable }
            guard current(), !Task.isCancelled else { return }
            try nativeLink.send(response)
        } catch {
            guard current(), !Task.isCancelled else { return }
            transmit(.object(["t": .string("BlobUnavailable"), "handle": frame["handle"], "reason": .string("unavailable")]))
        }
    }






    private func nativeReject(_ frame: JSON, error: any Error, connection: NativeSessionLink) async {
        let details = await rejection(frame, error: error)
        try? connection.send(NativeSessionFrame.rejected(commandID: frame["command_id"], code: details.code, message: details.message, evidence: details.evidence))
    }

    private static func rustOwned(_ operation: String) -> Bool {
        operation.hasPrefix("reminders.") || operation.hasPrefix("calendar.") || operation.hasPrefix("contacts.") || operation.hasPrefix("notes.")
    }

    private func nativePrimitive(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let connection = link else { return }
        let id = frame["id"]
        do {
            let operation = frame["op"].string
            let result: JSON
            if operation.hasPrefix("reminders.") { result = try await source.primitive(operation, args: frame["args"]) }
            else if operation.hasPrefix("calendar.") { result = try await calendar.primitive(operation, args: frame["args"]) }
            else if operation.hasPrefix("contacts.") { result = try await contacts.primitive(operation, args: frame["args"]) }
            else if operation.hasPrefix("messages."), let messages = additionalSources["messages"] as? Messages { result = try await messages.primitive(operation, args: frame["args"]) }
            else if operation.hasPrefix("notes."), let notes = additionalSources["notes"] as? NotesSource { result = try await notes.primitive(operation, args: frame["args"]) }
            else { throw SourceFailure.unsupported }
            if operation.hasSuffix(".write") || operation.hasSuffix(".write_group") {
                await sources.enumerations.invalidate(captureRoute(operation))
            }
            guard link === connection, !Task.isCancelled else { return }
            try connection.send(NativeSessionFrame.primitiveResult(id: id, result: result))
        } catch {
            guard link === connection, !Task.isCancelled else { return }
            let code: String
            if case SourceFailure.permissionMissing = error { code = "permission_missing" } else { code = errorCode(error) }
            try? connection.send(NativeSessionFrame.primitiveError(id: id, code: code))
        }
    }

    private func nativePrepare(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let connection = link else { return }
        do {
            _ = try CommandOrigin(frame)
            guard !frame["command_id"].string.isEmpty, !frame["principal"].string.isEmpty, !frame["request_hash"].string.isEmpty, let expires = parseTimestamp(frame["expires_at"].string), expires > Date() else { throw SourceFailure.invalid }
            guard !Self.rustOwned(frame["action"]["op"].string), let selected = additionalSources[captureRoute(frame["action"]["op"].string)] else { throw SourceFailure.unsupported }
            let prepared = try await selected.prepare(frame)
            guard !Task.isCancelled, link === connection, expires > Date() else { throw SourceFailure.expired }
            try connection.send(NativeSessionFrame.prepared(commandID: frame["command_id"], nativeState: prepared.nativeState, route: prepared.nativeState["route"].string.isEmpty ? "eventkit" : prepared.nativeState["route"].string, candidateIDs: prepared.nativeState["candidate_ids"].array, fingerprint: prepared.fingerprint))
        } catch { if link === connection && !Task.isCancelled { await nativeReject(frame, error: error, connection: connection) } }
    }

    private func nativeDispatch(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let connection = link else { return }
        let operation = frame["action"]["op"].string
        guard !Self.rustOwned(operation) else {
            try? connection.send(NativeSessionFrame.rejected(commandID: frame["command_id"], code: "unsupported", message: "unsupported", evidence: nil))
            return
        }
        let command: JSON = .object(["command_id": frame["command_id"], "request_hash": frame["request_hash"], "action": frame["action"]])
        let ticket = Ticket(command: command, prepared: Prepared(nativeState: frame["native_state"], fingerprint: frame["fingerprint"].string, action: frame["action"]), expiry: ContinuousClock.now.advanced(by: .seconds(30)), fence: fence)
        do {
            let evidence = try await executeValidated(operation, ticket: ticket, expiresAt: UInt64.max)
            guard link === connection, !Task.isCancelled else { return }
            let disposition = evidence["disposition"].string.isEmpty ? "locally_observed" : evidence["disposition"].string
            try connection.send(NativeSessionFrame.outcome(commandID: frame["command_id"], disposition: disposition, evidence: evidence))
        } catch let rejection as NativeEffectRejection {
            guard link === connection, !Task.isCancelled else { return }
            try? connection.send(NativeSessionFrame.rejected(commandID: frame["command_id"], code: rejection.code, message: rejection.message, evidence: rejection.evidence))
        } catch {
            guard link === connection, !Task.isCancelled else { return }
            try? connection.send(NativeSessionFrame.outcome(commandID: frame["command_id"], disposition: "indeterminate", evidence: .object(["reason": .string("dispatch_evidence_unavailable"), "observed": .bool(false)])))
        }
    }

    private func nativeResolve(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let connection = link else { return }
        let id = frame["command_id"].string
        let saved = frame["saved"]
        var disposition = saved["disposition"].string
        var evidence = saved["evidence"]
        do {
            if ["indeterminate", "locally_observed"].contains(disposition) {
                let operation = saved["evidence"]["prepared"]["action"]["op"].string
                guard !Self.rustOwned(operation) else { throw SourceFailure.unsupported }
                guard let selected = additionalSources[captureRoute(operation)] else { throw SourceFailure.unsupported }
                let resolved = await selected.resolve(id, evidence: saved["evidence"])
                if resolved["observed"].bool {
                    var result = resolved.object
                    result["fingerprint"] = saved["evidence"]["prepared"]["request_hash"]
                    result["prepared"] = saved["evidence"]["prepared"]
                    evidence = .object(result)
                    disposition = resolved["disposition"].string.isEmpty ? "locally_observed" : resolved["disposition"].string
                } else { disposition = "indeterminate" }
            }
            guard link === connection, !Task.isCancelled else { return }
            try connection.send(NativeSessionFrame.outcome(commandID: .string(id), disposition: disposition, evidence: evidence))
        } catch {
            guard link === connection, !Task.isCancelled else { return }
            try? connection.send(NativeSessionFrame.outcome(commandID: .string(id), disposition: "indeterminate", evidence: evidence))
        }
    }

    private func handleNative(_ frame: JSON) async throws {
        switch frame["t"].string {
        case "Prepare", "Dispatch", "Resolve", "BlobRead", "Primitive":
            launchFrame(frame)
        case "Query":
            let id = frame["id"].string
            guard !id.isEmpty, queryTasks[id] == nil else {
                transmit(.object(["t": .string("QueryError"), "id": frame["id"], "code": .string("conflict")]))
                return
            }
            queryTasks[id] = Task { await query(frame); queryTasks.removeValue(forKey: id) }
        case "QueryCancel": queryTasks[frame["id"].string]?.cancel()
        case "Close": throw SourceFailure.unavailable
        default: throw SourceFailure.invalid
        }
    }

    private func setFacts(_ server: NativeServer) { facts = server }

    public func runNative(socket: String, secretFile: String) async throws {
        let sessionHome = self.home.path
        try await runNative(connect: { [self] in
            let started = try NativeSessionLauncher.launch(socket: socket, secretFile: secretFile, home: sessionHome)
            let descriptor = started.proxy.factsDescriptor
            let facts = NativeServer(adapter: self)
            Task { await self.setFacts(facts) }
            let serving = Task {
                defer { Darwin.close(descriptor) }
                try? await facts.run(descriptor: descriptor)
            }
            let changes = Task { await self.forwardChanges(to: facts) }
            return (started.descriptor, {
                changes.cancel()
                serving.cancel()
                shutdown(descriptor, SHUT_RDWR)
                started.stop()
            })
        })
    }

    public func runNative(connect: @escaping @Sendable () throws -> (descriptor: Int32, stop: @Sendable () -> Void)) async throws {
        guard !running else { throw SourceFailure.conflict }
        running = true
        defer { running = false }
        for (name, source) in sources.additional { additionalSources[name] = source }
        var backoff = 100
        while !Task.isCancelled && kill(gatewayPID, 0) == 0 {
            var stop: (@Sendable () -> Void)?
            do {
                let started = try await Task.detached { try connect() }.value
                stop = started.stop
                let descriptor = started.descriptor
                let connected = try await Task.detached { try NativeSessionLink(descriptor: descriptor) }.value
                link = connected
                fence = connected.fence
                var coverage: [String: JSON] = [:]
                var capabilities: [String: JSON] = [:]
                for source in frameworkSources {
                    let state = await source.open()
                    coverage[source.name] = state.coverage
                    capabilities[source.name] = .array(state.capabilities.map(JSON.string))
                }
                try connected.send(NativeSessionFrame.ready(coverage: coverage, capabilities: capabilities, helperBuild: "m1-session", osBuild: ProcessInfo.processInfo.operatingSystemVersionString))
                try await publishReadiness(force: true)
                let watchdog = Task {
                    var delay = Duration.seconds(1)
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: delay) } catch { return }
                        if kill(self.gatewayPID, 0) != 0 { connected.close(); return }
                        let previous = self.permissionStates
                        try? await self.publishReadiness()
                        delay = previous == self.permissionStates ? min(.seconds(30), delay * 2) : .seconds(1)
                    }
                }
                defer { watchdog.cancel() }
                while !Task.isCancelled {
                    let frame = try await connected.receive()
                    try await handleNative(frame)
                    backoff = 100
                }
            } catch {
                disconnect()
            }
            stop?()
            guard kill(gatewayPID, 0) == 0 else { return }
            try await Task.sleep(for: .milliseconds(backoff))
            backoff = min(backoff * 2, 2000)
        }
    }



    func captureRoute(_ operation: String) -> String {
        if ["mac.screen_record", "print.document"].contains(operation) { return "capture" }
        return operation.components(separatedBy: ".").first ?? ""
    }

    private func errorCode(_ error: any Error) -> String {
        switch error {
        case SourceFailure.conflict: return "conflict"
        case SourceFailure.expired: return "expired"
        case SourceFailure.unsupported: return "unsupported"
 case SourceFailure.capabilityUnproven: return "capability_unproven"
        case SourceFailure.invalid: return "invalid"
        default: return "unavailable"
        }
    }

    struct Rejection {
        let code: String
        let message: String
        let evidence: JSON?
    }

    private func rejection(_ frame: JSON, error: any Error) async -> Rejection {
        let code = errorCode(error)
        let key = frame["action"]["native_key"].string
        if frame["action"]["op"].string.hasPrefix("messages.") {
            let detail = (error as? MessagesUIFailure)?.rawValue ?? code
            return Rejection(code: code, message: detail, evidence: nil)
        }
        if frame["action"]["op"].string.hasPrefix("notes.") { return Rejection(code: code, message: code, evidence: nil) }
        guard code == "conflict", !key.isEmpty else { return Rejection(code: code, message: code, evidence: nil) }
        return Rejection(code: "unavailable", message: "unavailable", evidence: nil)
    }

    private func durableAction(_ action: JSON) -> JSON {
        guard action["op"].string == "mac.clipboard_write" else { return action }
        var safe = action.object
        safe["args"] = .object([:])
        return .object(safe)
    }

    private func executeValidated(_ operation: String, ticket: Ticket, expiresAt: UInt64) async throws -> JSON {
        guard !commandActive else { throw NativeEffectRejection(code: "conflict", message: "conflict") }
        commandActive = true
        defer { commandActive = false }
        guard let selected = additionalSources[captureRoute(operation)] else { throw NativeEffectRejection(code: "unsupported", message: "unsupported") }
        do {
            guard ticket.fence == fence, connected, ContinuousClock.now < ticket.expiry,
                  expiresAt > UInt64(Date().timeIntervalSince1970) else { throw SourceFailure.expired }
        } catch {
            if error is CancellationError { throw error }
            let details = await rejection(ticket.command, error: error)
            throw NativeEffectRejection(code: details.code, message: details.message, evidence: details.evidence ?? .null)
        }
        let prepared: JSON = .object(["action": durableAction(ticket.prepared.action), "native_state": ticket.prepared.nativeState, "request_hash": ticket.command["request_hash"]])
        do {
            let result = try await selected.dispatch(ticket.prepared)
            if result["observed"].bool, operation == "pdf.annotate" { await facts?.changed(source: "pdf", generation: "native/1") }
            var fields = result.object
            fields["fingerprint"] = ticket.command["request_hash"]
            fields["before"] = ticket.prepared.nativeState
            fields["candidate_ids"] = ticket.prepared.nativeState["candidate_ids"]
            fields["prepared"] = prepared
            return .object(fields)
        } catch {
            if error is CancellationError { throw error }
            let nativeCode = (error as? SourceFailure).map { String(describing: $0) } ?? (error as? MessagesUIFailure)?.rawValue ?? "native_error"
            let reason = (error as? MailCommandFailure)?.localizedDescription ?? "dispatch_evidence_unavailable"
            return .object(["disposition": .string("indeterminate"), "observed": .bool(false), "reason": .string(reason), "native_error_code": .string(nativeCode), "prepared": prepared])
        }
    }


}
