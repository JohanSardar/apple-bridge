import NativeProtocol
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
    private var running = false
    var link: NativeSessionLink?
    private var facts: NativeServer?
    private var connected: Bool { link != nil }
    private var lastStatus: JSON = .null
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
        lastStatus = .null
        for task in frameTasks.values { task.cancel() }
        frameTasks = [:]
        for task in queryTasks.values { task.cancel() }; queryTasks = [:]
    }

    private var allSources: [any Source] { [source, calendar] + additionalSources.values }

    private func frameworkStatus() async -> JSON {
        var status: [String: JSON] = [:]
        for source in allSources {
            if let fields = await source.status() { status[source.name] = fields }
        }
        return .object(status)
    }

    private func publishStatus(on connected: NativeSessionLink) async throws {
        let status = await frameworkStatus()
        guard status != lastStatus else { return }
        lastStatus = status
        try connected.send(.object(["t": .string("FrameworkStatus"), "status": status]))
    }

    private func launchFrame(_ frame: JSON) {
        let id = UUID()
        frameTasks[id] = Task {
            switch frame["t"].string {
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
            else { throw SourceFailure.unavailable }
            guard current(), !Task.isCancelled else { return }
            try nativeLink.send(response)
        } catch {
            guard current(), !Task.isCancelled else { return }
            transmit(.object(["t": .string("BlobUnavailable"), "handle": frame["handle"], "reason": .string("unavailable")]))
        }
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
            else if let selected = additionalSources[captureRoute(operation)] { result = try await selected.primitive(operation, args: frame["args"]) }
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

    private func handleNative(_ frame: JSON) async throws {
        switch frame["t"].string {
        case "BlobRead", "Primitive":
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
                try connected.send(NativeSessionFrame.ready(framework: await frameworkStatus(), helperBuild: "m1-session", osBuild: ProcessInfo.processInfo.operatingSystemVersionString))
                try await publishStatus(on: connected)
                let watchdog = Task {
                    var delay = Duration.seconds(1)
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: delay) } catch { return }
                        if kill(self.gatewayPID, 0) != 0 { connected.close(); return }
                        let previous = self.lastStatus
                        try? await self.publishStatus(on: connected)
                        delay = previous == self.lastStatus ? min(.seconds(30), delay * 2) : .seconds(1)
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
}
