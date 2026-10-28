import Foundation
import Darwin
import SourceSupport
import AgendaGate
import RemindersSource
import RemindersStore
import NotesSource
import ContactsSource
import UsageSource
import CalendarSource
import MessagesSource

public actor AppleSession: ObservationSink {
    private struct Pending {
        let source: String
        let checkpoint: String
        let generation: String
        let scanID: String
        let completionEvidence: String
        let blobHandles: [String: String]
        let observations: [JSON]
        let started: String
        let scope: [String]
        let completion: CheckedContinuation<Void, any Error>
    }
    private struct Flight {
        let pending: Pending
        let checkpoint: String
        let bytes: Int
        let prioritized: Bool
    }
    private struct Ticket {
        let command: JSON
        let prepared: Prepared
        let expiry: ContinuousClock.Instant
        let fence: UInt64
    }
    private var draining = false
    private var wire: PeerWire?
    private var fence: UInt64 = 0
    private var permissionStates: [String: String] = [:]
    private var credit = 0
    private var checkpoints: [String: JSON] = [:]
    private var observedCheckpoints: [String: String] = [:]
    private var live: [Pending] = []
    private var bulk: [Pending] = []
    private var flights: [String: Flight] = [:]
    private var tickets: [String: Ticket] = [:]
    private var preparing: UUID?
    private var commandActive = false
    private var additionalSources: [String: any Source] = [:]
    private var blobReader: (@Sendable (String, Int64, Int) async throws -> JSON)?
    private var captureTasks: [Task<Void, Never>] = []
    private var queryHandlers: [String: @Sendable (JSON) async throws -> JSON] = [:]
    public func registerQuery(_ name: String, handler: @escaping @Sendable (JSON) async throws -> JSON) throws {
        guard wire == nil, queryHandlers[name] == nil else { throw SourceFailure.conflict }
        queryHandlers[name] = handler
    }
    private var queryTasks: [String: Task<Void, Never>] = [:]
    private var frameTasks: [UUID: Task<Void, Never>] = [:]
    private let trace: CaptureTrace
    private let sources: SessionSources.Builtins
    private var contacts: ContactsSource { sources.contacts }
    private var source: RemindersSource { sources.reminders }
    private var remindersStore: RemindersStoreEnrichment { sources.remindersStore }
    private var calendar: CalendarSource { sources.calendar }
    private var remindersRefreshHistogram = StoreRefreshHistogram()
    private let guardLog: DispatchGuard
    private let socket: String
    private let secretFile: String
    private let gatewayPID: Int32
    private let home: URL
    private let labEnabled: Bool

    @MainActor
    public init(socket: String, secretFile: String, home: URL, gatewayPID: Int32, commandsEnabled: Bool) throws {
        self.socket = socket; self.secretFile = secretFile; self.gatewayPID = gatewayPID
        self.home = home; self.labEnabled = commandsEnabled
        let trace = CaptureTrace(url: commandsEnabled ? home.appendingPathComponent("capture-trace.jsonl") : nil)
        self.trace = trace
        sources = try SessionSources.Builtins(home: home, labEnabled: commandsEnabled, trace: trace)
        guardLog = try DispatchGuard(home: home)
        let pidFile = home.appendingPathComponent("helper.pid")
        try Data(String(getpid()).utf8).write(to: pidFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pidFile.path)
    }

    public func register(_ source: any Source, readBlob: (@Sendable (String, Int64, Int) async throws -> JSON)? = nil) throws {
        guard wire == nil, source.name != "reminders", additionalSources[source.name] == nil else { throw SourceFailure.conflict }
        additionalSources[source.name] = source
        if let readBlob { blobReader = readBlob }
    }

    public func run() async throws {
        for (name, source) in sources.additional { additionalSources[name] = source }
        var backoff = 100
        while !Task.isCancelled && kill(gatewayPID, 0) == 0 {
            do {
                let socket = self.socket, secretFile = self.secretFile
                let connected = try await Task.detached { try PeerWire(socket: socket, secretFile: secretFile) }.value
                wire = connected
                let opened = try await receive(connected)
                guard opened["t"].string == "Open", opened["protocol"].string == "ipc/1", opened["owner_fence"].number > 0, opened["byte_credit"].number > 0, opened["byte_credit"].number <= 4194304 else { throw SourceFailure.invalid }
                fence = opened["owner_fence"].unsigned; credit = Int(opened["byte_credit"].number); checkpoints = opened["checkpoints"].object
                let readiness = await source.open()
                let calendarReadiness=await calendar.open()
 var coverage: [String: JSON] = ["reminders": readiness.coverage, "calendar": calendarReadiness.coverage]
                var capabilities: [String: JSON] = ["reminders": .array(readiness.capabilities.map(JSON.string)), "calendar": .array(calendarReadiness.capabilities.map(JSON.string))]
                for (name, source) in additionalSources {
                    let state = await source.open()
                    coverage[name] = state.coverage
                    capabilities[name] = .array(state.capabilities.map(JSON.string))
                }
                try connected.send(.object(["t": .string("Ready"), "protocol": .string("ipc/1"), "coverage": .object(coverage), "capabilities": .object(capabilities), "helper_build": .string("m1-session"), "os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString)]))
                try await publishReadiness()
                for outcome in await guardLog.outcomes.values where outcome["disposition"].string == "indeterminate" { try connected.send(outcome) }
                captureTasks.append(Task { await self.capture() })
 captureTasks.append(Task {await self.captureAdditional(calendar)})
                for source in additionalSources.values { captureTasks.append(Task { await self.captureAdditional(source) }) }
                let watchdog = Task {
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .seconds(1)) } catch { return }
                        if kill(self.gatewayPID, 0) != 0 { connected.close(); return }
                        try? await self.publishReadiness()
                    }
                }
                defer { watchdog.cancel() }
                while !Task.isCancelled {
                    let frame = try await receive(connected)
                    try await handle(frame)
                    if frame["t"].string == "Committed" { backoff = 100 }
                }
            } catch {
                disconnect()
            }
            guard kill(gatewayPID, 0) == 0 else { return }
            try await Task.sleep(for: .milliseconds(backoff))
            backoff = min(backoff * 2, 2000)
        }
    }

    private func receive(_ wire: PeerWire) async throws -> JSON {
        try await Task.detached { try wire.receive() }.value
    }

    private func disconnect() {
        wire?.close(); wire = nil
        permissionStates = [:]
        observedCheckpoints = [:]
        for task in captureTasks { task.cancel() }
        captureTasks = []
        for task in frameTasks.values { task.cancel() }
        frameTasks = [:]
        for task in queryTasks.values { task.cancel() }; queryTasks = [:]
        for pending in live + bulk { pending.completion.resume(throwing: SourceFailure.unavailable) }
        for flight in flights.values { flight.pending.completion.resume(throwing: SourceFailure.unavailable) }
        live = []; bulk = []; flights = [:]; tickets = [:]; preparing = nil
        trace.record("capture_session_disconnected")
    }

    public func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        guard let wire else { throw SourceFailure.unavailable }
        try wire.send(.object(["t": .string("SourceState"), "source": .string(source), "state": .string(state), "coverage": .object([source: coverage]), "capabilities": .object([source: .array(capabilities.map(JSON.string))]), "reason": .string(reason)]))
        Task { do { try await drain() } catch { disconnect() } }
    }

    private func publishReadiness() async throws {
        let readiness = await source.open()
        if readiness.state != permissionStates["reminders"] {
            permissionStates["reminders"] = readiness.state
            try await sourceState(source: "reminders", state: readiness.state, coverage: readiness.coverage, capabilities: readiness.capabilities, reason: readiness.reason)
        }
        let calendarReadiness = await calendar.open()
        if calendarReadiness.state != permissionStates["calendar"] {
            permissionStates["calendar"] = calendarReadiness.state
            try await sourceState(source: "calendar", state: calendarReadiness.state, coverage: calendarReadiness.coverage, capabilities: calendarReadiness.capabilities, reason: calendarReadiness.reason)
        }
    }

    private func captureResume(_ name: String) -> JSON {
        let checkpoint = checkpoints[name] ?? .null
        return checkpoint["refresh_blobs"].bool ? .null : checkpoint
    }

    private func capture() async {
        while !Task.isCancelled && wire != nil {
            do {
                try await publishReadiness()
                let readiness = await source.open()
                if readiness.state == "ready" { try await source.capture(captureResume("reminders"), into: self) }
            } catch {
                if wire == nil || Task.isCancelled { return }
                if case SourceFailure.permissionMissing = error {
                    try? await publishReadiness()
                } else {
                    try? await sourceState(source: "reminders", state: "degraded", coverage: RemindersSource.coverage, capabilities: [], reason: "capture_failed")
                }
                if !(error is SourceFailure) { wire?.close(); return }
            }
            do { try await Task.sleep(for: .seconds(1)) } catch { return }
        }
    }

    private func captureAdditional(_ source: any Source) async {
        while !Task.isCancelled && wire != nil {
            do {
                let readiness = await source.open()
                try await sourceState(source: source.name, state: readiness.state, coverage: readiness.coverage, capabilities: readiness.capabilities, reason: readiness.reason)
                try await source.capture(captureResume(source.name), into: self)
            } catch {
                if Task.isCancelled || wire == nil { return }
                let permissionMissing: Bool
                if case SourceFailure.permissionMissing = error { permissionMissing = true } else { permissionMissing = false }
                let readiness = await source.open()
                try? await sourceState(source: source.name, state: permissionMissing ? "permission_missing" : "degraded", coverage: readiness.coverage, capabilities: [], reason: permissionMissing ? "permission_required" : "capture_failed")
            }
            do { try await Task.sleep(for: .seconds(1)) } catch { return }
        }
    }

    private func launchFrame(_ frame: JSON) {
        let id = UUID()
        frameTasks[id] = Task {
            switch frame["t"].string {
            case "Prepare": await prepare(frame)
            case "Dispatch": await dispatch(frame)
            case "Resolve": await resolve(frame)
            case "BlobRead": await readBlob(frame)
            default: break
            }
            frameTasks.removeValue(forKey: id)
        }
    }

    private func query(_ frame: JSON) async {
        guard !Task.isCancelled else { return }
        let id = frame["id"]
        if let handler = queryHandlers[frame["op"].string] {
            do {
                let result = try await handler(frame["args"])
                guard !Task.isCancelled else { return }
                let encoded = try JSONEncoder().encode(result)
                guard encoded.count <= 1 << 20 else { throw SourceFailure.invalid }
                try? wire?.send(.object(["t": .string("QueryResult"), "id": id, "result_json": result, "blobs": .array([])]))
            } catch {
                let code = error is CancellationError ? "deadline" : (error as? SourceFailure == .invalid ? "invalid" : "unavailable")
                try? wire?.send(.object(["t": .string("QueryError"), "id": id, "code": .string(code)]))
            }
            return
        }
        guard frame["op"].string == "helper.version" else {
            try? wire?.send(.object(["t": .string("QueryError"), "id": id, "code": .string("unsupported")]))
            return
        }
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development"
        let result = JSON.object(["version": .string(version), "helper_build": .string("m1-session"), "protocol": .string("ipc/1")])
        guard !Task.isCancelled else { return }
        try? wire?.send(.object(["t": .string("QueryResult"), "id": id, "result_json": result, "blobs": .array([])]))
    }

    private func readBlob(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let connection = wire else { return }
        let started = CaptureTrace.now()
        var prepared: UInt64 = 0
        let source = frame["handle"].string.hasPrefix("notes-blob:") ? "notes" : (frame["handle"].string.hasPrefix("contacts-image:") ? "contacts" : (frame["handle"].string.hasPrefix("calendar-attachment:") ? "calendar" : "messages"))
        defer { trace.record("helper_blob_read", fields: ["source": .string(source), "started_ns": .integer(started), "prepared_ns": .integer(prepared), "finished_ns": .integer(CaptureTrace.now())]) }
        do {
            guard !frame["handle"].string.isEmpty, frame["offset"].number >= 0, frame["offset"].number < Double(Int64.max), frame["length"].number > 0, frame["length"].number <= 131072, frame["length"].number.rounded(.down) == frame["length"].number, frame["offset"].number.rounded(.down) == frame["offset"].number else { throw SourceFailure.invalid }
            let response: JSON
            if frame["handle"].string.hasPrefix("notes-blob:") {
                guard let notes = additionalSources["notes"] as? NotesSource else { throw SourceFailure.unavailable }
                response = try await notes.readBlob(frame["handle"].string, offset: Int(frame["offset"].number), length: Int(frame["length"].number))
            }
            else if frame["handle"].string.hasPrefix("contacts-image:") { response = try await contacts.readBlob(frame["handle"].string, offset: Int(frame["offset"].number), length: Int(frame["length"].number)) }
            else if frame["handle"].string.hasPrefix("calendar-attachment:") { response = try await calendar.readBlob(frame["handle"].string, offset: Int(frame["offset"].number), length: Int(frame["length"].number)) }
            else if let blobReader { response = try await blobReader(frame["handle"].string, Int64(frame["offset"].number), Int(frame["length"].number)) }
            else { throw SourceFailure.unavailable }
            guard wire === connection, !Task.isCancelled else { return }
            prepared = CaptureTrace.now()
            try await connection.sendBlob(response, trace: trace, source: source)
        } catch {
            guard wire === connection, !Task.isCancelled else { return }
            try? connection.send(.object(["t": .string("BlobUnavailable"), "handle": frame["handle"], "reason": .string("unavailable")]))
        }
    }

    public func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard wire != nil else { throw SourceFailure.unavailable }
        try await emitCheckpoint(source: source, generation: generation, checkpoint: "", observations: observations, scanID: scanID, scanStartedAt: scanStartedAt, scope: scope, completionEvidence: completionEvidence, blobHandles: blobHandles, live: live)
    }

    public func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard wire != nil else { throw SourceFailure.unavailable }
        let metadata: [String: JSON] = ["source": .string(source), "scan_id": .string(scanID)]
        trace.record("capture_received", fields: metadata)
        let connection = wire
        let refreshStarted = source == "reminders" && labEnabled ? CaptureTrace.now() : 0
        let observations = source == "reminders" ? await remindersStore.refresh(observations, generation: generation) : observations
        let refreshFinished = source == "reminders" && labEnabled ? CaptureTrace.now() : 0
        guard wire === connection, !Task.isCancelled else { throw SourceFailure.unavailable }
        if source == "reminders", labEnabled {
            let milliseconds = Double(refreshFinished - refreshStarted) / 1_000_000
            trace.record("reminders_store_refresh", keys: observations.map { $0["native_key"].string }, fields: ["duration_ms": .number(milliseconds), "started_ns": .integer(refreshStarted), "finished_ns": .integer(refreshFinished), "histogram": remindersRefreshHistogram.record(milliseconds)])
        }
        let maximumBytes = live ? (source == "calendar" ? 240000 : 64000) : 16000
        let chunks = try split(observations, maximumBytes: maximumBytes)
        trace.record("capture_split", fields: metadata)
        for (index,chunk) in chunks.enumerated() {
            try await withCheckedThrowingContinuation { continuation in
                trace.record("capture_chunk_preparing", fields: metadata)
                let pending = Pending(source: source, checkpoint: index == chunks.count-1 ? checkpoint : "", generation: generation, scanID: scanID, completionEvidence: completionEvidence, blobHandles: blobHandles.filter { digest, _ in Self.containsDigest(.array(chunk), digest: digest) }, observations: chunk, started: scanStartedAt, scope: scope, completion: continuation)
                trace.record("capture_enqueued", fields: ["source": .string(source), "scan_id": .string(scanID)])
                if live { self.live.append(pending) } else { bulk.append(pending) }
                Task { do { try await drain() } catch { disconnect() } }
            }
        }
    }

    private static func containsDigest(_ value: JSON, digest: String) -> Bool {
        switch value {
        case .object(let fields): return fields["digest"]?.string == digest || fields.values.contains { containsDigest($0, digest: digest) }
        case .array(let values): return values.contains { containsDigest($0, digest: digest) }
        default: return false
        }
    }

    private func split(_ observations: [JSON], maximumBytes: Int) throws -> [[JSON]] {
        var chunks: [[JSON]] = [], current: [JSON] = []
        var bytes = 0
        let ordered = observations.filter { $0["kind"].string != "absent" } + observations.filter { $0["kind"].string == "absent" }
        for observation in ordered {
            let count = try JSONEncoder().encode(observation).count + 1
            guard count < 240000 else { throw SourceFailure.invalid }
            if !current.isEmpty && bytes + count > maximumBytes { chunks.append(current); current = []; bytes = 0 }
            current.append(observation); bytes += count
        }
        if !current.isEmpty || chunks.isEmpty { chunks.append(current) }
        return chunks
    }

    private func drain() async throws {
        guard !draining else { return }
        draining = true
        defer { draining = false }
        while !live.isEmpty || !bulk.isEmpty {
            let liveIndex = live.firstIndex { flights[$0.source] == nil }
            let bulkIndex = flights.values.contains(where: { !$0.prioritized }) ? nil : bulk.firstIndex { flights[$0.source] == nil }
            guard let index = liveIndex ?? bulkIndex else { return }
            let prioritized = liveIndex != nil
            let pending = prioritized ? live[index] : bulk[index]
            if !prioritized && labEnabled && FileManager.default.fileExists(atPath: home.appendingPathComponent("lab-hold-backfill").path) {
                try Data("held".utf8).write(to: home.appendingPathComponent("lab-backfill-held"), options: .atomic)
                return
            }
            let connection = wire
            let observations: [JSON]
            if !prioritized && pending.source == "reminders" {
                let validated = try await source.revalidate(pending.observations, generation: pending.generation)
                guard wire === connection, !Task.isCancelled else { return }
                guard let validated else {
                    if live.contains(where: { flights[$0.source] == nil }) { continue }
                    return
                }
                observations = await remindersStore.apply(validated)
                guard wire === connection, !Task.isCancelled else { return }
                if live.contains(where: { flights[$0.source] == nil }) { continue }
            } else { observations = pending.observations }
            let sending = Pending(source: pending.source, checkpoint: pending.checkpoint, generation: pending.generation, scanID: pending.scanID, completionEvidence: pending.completionEvidence, blobHandles: pending.blobHandles, observations: observations, started: pending.started, scope: pending.scope, completion: pending.completion)
            let checkpoint = pending.checkpoint.isEmpty ? UUID().uuidString : pending.checkpoint
            let resume = checkpoints[pending.source] ?? .null
            var capture: [String: JSON] = ["source": .string(pending.source), "generation": .string(pending.generation), "predecessor": .string(resume["generation"].string == pending.generation ? resume["checkpoint"].string : ""), "checkpoint": .string(checkpoint), "observations": .array(observations)]
            if !pending.scanID.isEmpty {
                capture["scan_id"] = .string(pending.scanID); capture["scope"] = .array(pending.scope.map(JSON.string)); capture["scan_started_at"] = .string(pending.started.isEmpty ? timestamp() : pending.started); capture["completion_evidence"] = .string(pending.completionEvidence)
            }
            capture["blob_handles"] = .object(pending.blobHandles.mapValues(JSON.string))
            capture["live"] = .bool(prioritized)
            capture["t"] = .string("Capture")
            let frame = JSON.object(capture)
            let size = try PeerWire.encode(frame).count
            guard size <= credit else { return }
            trace.record("capture_sent", keys: observations.map { $0["native_key"].string }, fields: ["source": .string(pending.source), "scan_id": .string(pending.scanID), "checkpoint": .string(checkpoint), "live": .bool(prioritized), "credit": .number(Double(credit)), "blob_count": .number(Double(pending.blobHandles.count))])
            try wire?.send(frame)
            trace.record("capture_written", fields: ["source": .string(pending.source), "scan_id": .string(pending.scanID)])
            credit -= size
            flights[pending.source] = Flight(pending: sending, checkpoint: checkpoint, bytes: size, prioritized: prioritized)
            if prioritized { live.remove(at: index) } else { bulk.remove(at: index) }
        }
    }

    private func handle(_ frame: JSON) async throws {
        switch frame["t"].string {
        case "Ping":
            try wire?.send(.object(["t": .string("Pong")]))
            try await drain()
        case "Pong": break
        case "Committed":
            let source = frame["source"].string
            guard let flight = flights[source], frame["checkpoint"].string == flight.checkpoint, frame["credit"].unsigned == UInt64(flight.bytes) else { throw SourceFailure.invalid }
            trace.record("committed_ack", keys: flight.pending.observations.map { $0["native_key"].string }, fields: ["source": .string(source), "scan_id": .string(flight.pending.scanID), "checkpoint": .string(flight.checkpoint)])
            try await pause("capture", observations: flight.pending.observations)
            credit += flight.bytes
            checkpoints[source] = .object(["generation": .string(flight.pending.generation), "checkpoint": .string(flight.checkpoint)])
            if labEnabled {
                let markerStarted = CaptureTrace.now()
                defer { trace.record("helper_marker_written", fields: ["source": .string(source), "started_ns": .integer(markerStarted), "finished_ns": .integer(CaptureTrace.now())]) }
                let marker = home.appendingPathComponent("lab-committed-" + source)
                try Data(flight.checkpoint.utf8).write(to: marker, options: .atomic)
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: marker.path)
            }
            for observation in flight.pending.observations where ["upsert", "absent"].contains(observation["kind"].string) {
                observedCheckpoints[observation["collection"].string + "/" + observation["native_key"].string] = flight.checkpoint
            }
            if source == "reminders" { try await self.source.didCommit(flight.pending.observations.map(RemindersStoreProjection.baseObservation), generation: flight.pending.generation) }
            flights.removeValue(forKey: source)
            flight.pending.completion.resume()
            try await drain()
        case "Reset": throw SourceFailure.conflict
        case "Query":
            let id = frame["id"].string
            guard !id.isEmpty, queryTasks[id] == nil else {
                try? wire?.send(.object(["t": .string("QueryError"), "id": frame["id"], "code": .string("conflict")]))
                return
            }
            queryTasks[id] = Task { await query(frame); queryTasks.removeValue(forKey: id) }
        case "QueryCancel": queryTasks[frame["id"].string]?.cancel()
        case "Prepare": launchFrame(frame)
        case "Dispatch": launchFrame(frame)
        case "Resolve": launchFrame(frame)
        case "BlobRead":
            launchFrame(frame)
        case "Close": throw SourceFailure.unavailable
        default: throw SourceFailure.invalid
        }
    }

    private func observedEvidence(_ evidence: JSON, fingerprint: JSON, operation: String) async throws -> JSON {
        if operation.hasPrefix("notes."), let notes=additionalSources["notes"] as? NotesSource {
            try await notes.reconcile(into:self)
            let key=evidence["native_key"].string
            guard let checkpoint=observedCheckpoints["notes.note/"+key] ?? observedCheckpoints["notes.folder/"+key] else {throw SourceFailure.unavailable}
            var result=evidence.object;result["fingerprint"]=fingerprint;result["capture_checkpoint"] = .string(checkpoint)
            return .object(result)
        }
        let calendarCommand = operation.hasPrefix("calendar.")
        if operation.hasPrefix("contacts.") {
            trace.record("contacts_command_reconcile_requested", keys: [evidence["native_key"].string], fields: ["operation": .string(operation)])
            try await contacts.reconcile(into: self)
            if evidence["absent"].bool { try await contacts.confirmAbsence(evidence["native_key"].string, into: self) }
            guard let checkpoint = observedCheckpoints["contacts.contact/" + evidence["native_key"].string] else { throw SourceFailure.unavailable }
            var result = evidence.object
            result["fingerprint"] = fingerprint; result["capture_checkpoint"] = .string(checkpoint)
            return .object(result)
        }
 if evidence["source"].string == "messages" {
 guard let messages=additionalSources["messages"] as? Messages else {throw SourceFailure.unavailable}
 try await messages.captureCommand(evidence, into:self)
 let collection=evidence["collection"].string.isEmpty ? "messages.message" : evidence["collection"].string
 guard let checkpoint=observedCheckpoints[collection+"/"+evidence["native_key"].string] else {throw SourceFailure.unavailable}
 var result=evidence.object; result["fingerprint"]=fingerprint; result["capture_checkpoint"] = .string(checkpoint)
 return .object(result)
 }
        try Task.checkCancellation()
        if calendarCommand {
 try await calendar.reconcile(into:self)
 if evidence["absent"].bool {try await calendar.confirmAbsence(evidence["native_key"].string,into:self)}
 }else{
 try await source.reconcile(into: self)
 if evidence["absent"].bool { try await source.confirmAbsence(evidence["native_key"].string, into: self) }
 }
        try Task.checkCancellation()
        let key = evidence["native_key"].string
        guard let checkpoint = observedCheckpoints[(calendarCommand ? "calendar.event/" : "reminders.reminder/") + key] ?? observedCheckpoints[(calendarCommand ? "calendar.calendar/" : "reminders.list/") + key] else { throw SourceFailure.unavailable }
        var result = evidence.object
        result["fingerprint"] = fingerprint
        result["capture_checkpoint"] = .string(checkpoint)
        return .object(result)
    }

    private func resolve(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        let id = frame["command_id"].string
        do {
            guard var outcome = await guardLog.outcomes[id] else { try reject(frame, code: "not_found"); return }
            if ["indeterminate", "locally_observed"].contains(outcome["disposition"].string) {
let operation=outcome["evidence"]["prepared"]["action"]["op"].string
                let saved=outcome["evidence"]
 let selected=additionalSources[saved["prepared"]["action"]["op"].string.components(separatedBy:".").first ?? ""] ?? source
 let evidence = operation.hasPrefix("contacts.") ? await contacts.resolve(id,evidence:saved) : operation.hasPrefix("calendar.") ? await calendar.resolve(id,evidence:saved) : await selected.resolve(id, evidence: saved)
                if evidence["observed"].bool {
                    let committed = try await observedEvidence(evidence, fingerprint: outcome["evidence"]["prepared"]["request_hash"], operation: operation)
                    outcome = .object(["t": .string("Outcome"), "command_id": .string(id), "disposition": evidence["disposition"].string.isEmpty ? .string("locally_observed") : evidence["disposition"], "evidence": committed])
                    try await guardLog.append(outcome)
                } else {
                    var unresolved = outcome.object
                    unresolved["disposition"] = .string("indeterminate")
                    outcome = .object(unresolved)
                }
            }
            try wire?.send(outcome)
        } catch { }
    }

    private func pause(_ point: String, observations: [JSON] = []) async throws {
        guard labEnabled else { return }
        let marker = home.appendingPathComponent("lab-pause-" + point)
        guard FileManager.default.fileExists(atPath: marker.path) else { return }
        if point == "capture", let title = try? String(contentsOf: marker, encoding: .utf8), title.hasPrefix("[ab-test "), !observations.contains(where: { $0["data"]["title"].string == title }) { return }
        let signal = home.appendingPathComponent("lab-paused-" + point)
        try Data("paused".utf8).write(to: signal, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: signal.path)
        while FileManager.default.fileExists(atPath: marker.path) {
            guard kill(gatewayPID, 0) == 0 else { throw SourceFailure.unavailable }
            try await Task.sleep(for: .milliseconds(10))
        }
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

    private func reject(_ frame: JSON, code: String) throws {
        try wire?.send(.object(["t": .string("Rejected"), "command_id": frame["command_id"], "disposition": .string("rejected_before_dispatch"), "error": .object(["code": .string(code), "message": .string(code)])]))
    }

    private func rejectCommand(_ frame: JSON, error: any Error) async {
        let code = errorCode(error)
        let key = frame["action"]["native_key"].string
        if frame["action"]["op"].string.hasPrefix("messages.") {
            let detail = (error as? MessagesUIFailure)?.rawValue ?? code
            try? wire?.send(.object(["t": .string("Rejected"), "command_id": frame["command_id"], "disposition": .string("rejected_before_dispatch"), "error": .object(["code": .string(code), "message": .string(detail)])]))
            return
        }
        if frame["action"]["op"].string.hasPrefix("notes.") {try? reject(frame,code:code);return}
        guard code == "conflict", !key.isEmpty else { try? reject(frame, code: code); return }
        do {
            let isContact = frame["action"]["op"].string.hasPrefix("contacts.")
            if isContact { try await contacts.reconcile(into: self, key: key) } else if frame["action"]["op"].string.hasPrefix("calendar.") { try await calendar.reconcile(into: self) } else { try await source.reconcile(into: self) }
            guard let checkpoint = observedCheckpoints[(isContact ? "contacts.contact/" : frame["action"]["op"].string.hasPrefix("calendar.") ? "calendar.event/" : "reminders.reminder/") + key] else { throw SourceFailure.unavailable }
            let evidence = JSON.object(["native_key": .string(key), "capture_checkpoint": .string(checkpoint)])
            try wire?.send(.object(["t": .string("Rejected"), "command_id": frame["command_id"], "disposition": .string("rejected_before_dispatch"), "error": .object(["code": .string(code), "message": .string(code)]), "evidence": evidence]))
        } catch { try? reject(frame, code: "unavailable") }
    }

    private func prepare(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        let nonce = UUID()
        guard let connection = wire else { return }
        let owner = fence
        defer { if preparing == nonce { preparing = nil } }
        do {
            _ = try CommandOrigin(frame)
            guard !frame["command_id"].string.isEmpty, !frame["principal"].string.isEmpty, !frame["request_hash"].string.isEmpty, let expires = parseTimestamp(frame["expires_at"].string), expires > Date() else { throw SourceFailure.invalid }
            tickets = tickets.filter { ContinuousClock.now < $0.value.expiry }
            guard !commandActive, tickets.isEmpty, preparing == nil else { throw SourceFailure.conflict }
            preparing = nonce
            guard !(await guardLog.contains(frame["command_id"].string)) else { throw SourceFailure.conflict }
            let selected = additionalSources[frame["action"]["op"].string.components(separatedBy: ".").first ?? ""] ?? source
 var nativeFrame = frame.object
            if frame["action"]["op"].string.hasPrefix("reminders.") { nativeFrame["expected_state"] = RemindersStoreProjection.baseData(frame["expected_state"]) }
 let prepared = frame["action"]["op"].string.hasPrefix("contacts.") ? try await contacts.prepare(frame) : frame["action"]["op"].string.hasPrefix("calendar.") ? try await calendar.prepare(frame) : try await selected.prepare(.object(nativeFrame))
            guard !Task.isCancelled, wire === connection, fence == owner, preparing == nonce, expires > Date() else { throw SourceFailure.expired }
            let ticket = UUID().uuidString
            tickets[ticket] = Ticket(command: frame, prepared: prepared, expiry: ContinuousClock.now.advanced(by: .milliseconds(Int64(min(30, expires.timeIntervalSinceNow) * 1000))), fence: owner)
            try wire?.send(.object(["t": .string("Prepared"), "command_id": frame["command_id"], "ticket": .string(ticket), "native_state": prepared.nativeState, "route": prepared.nativeState["route"].string.isEmpty ? .string("eventkit") : prepared.nativeState["route"], "fingerprint": frame["request_hash"], "candidate_ids": prepared.nativeState["candidate_ids"].array.isEmpty ? .array([]) : prepared.nativeState["candidate_ids"]]))
        } catch { if wire === connection && !Task.isCancelled { await rejectCommand(frame, error: error) } }
    }

    private func dispatch(_ frame: JSON) async {
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        guard let ticket = tickets.removeValue(forKey: frame["ticket"].string) else { try? reject(frame, code: "conflict"); return }
        let id = ticket.command["command_id"]
        guard !commandActive else { try? reject(ticket.command, code: "conflict"); return }
        commandActive = true
        defer { commandActive = false }
        var started = false
        do {
            guard frame["origin"] == ticket.command["origin"], frame["command_id"] == id, frame["owner_fence"].unsigned == fence, ticket.fence == fence, ContinuousClock.now < ticket.expiry else { throw SourceFailure.expired }
            let selected=additionalSources[ticket.prepared.action["op"].string.components(separatedBy: ".").first ?? ""] ?? source
 if ticket.prepared.action["op"].string.hasPrefix("notes."), let notes=additionalSources["notes"] as? NotesSource { try await notes.validate(ticket.prepared) } else if ticket.prepared.action["op"].string.hasPrefix("contacts.") {try await contacts.validate(ticket.prepared)} else if ticket.prepared.action["op"].string.hasPrefix("calendar.") {try await calendar.validate(ticket.prepared)} else if selected.name == "reminders" { try await source.validate(ticket.prepared) } else if let messages=selected as? Messages {try await messages.validate(ticket.prepared)}
            try Task.checkCancellation()
            guard ticket.fence == fence, ContinuousClock.now < ticket.expiry, wire != nil else { throw SourceFailure.expired }
            try await guardLog.append(.object(["t": .string("DispatchStarted"), "command_id": id, "ticket": frame["ticket"], "started_at": .string(timestamp()), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "fingerprint": .string(ticket.prepared.fingerprint), "request_hash": ticket.command["request_hash"]])]))
            started = true
            try await pause("dispatch")
            try Task.checkCancellation()
            guard ticket.fence == fence, ContinuousClock.now < ticket.expiry, wire != nil else { throw SourceFailure.conflict }
            let refetched = ticket.prepared.action["op"].string.hasPrefix("contacts.") ? try await contacts.dispatch(ticket.prepared) : ticket.prepared.action["op"].string.hasPrefix("calendar.") ? try await calendar.dispatch(ticket.prepared) : try await selected.dispatch(ticket.prepared)
            try await pause("outcome")
            var retained = refetched.object
            retained["prepared"] = .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "request_hash": ticket.command["request_hash"]])
            let evidence = refetched["observed"].bool ? try await observedEvidence(.object(retained), fingerprint: ticket.command["request_hash"], operation:ticket.prepared.action["op"].string) : .object(retained)
            let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": refetched["disposition"].string.isEmpty ? .string("locally_observed") : refetched["disposition"], "evidence": evidence])
            try await guardLog.append(outcome)
            if selected.name == "messages" {
                let readiness=await selected.open()
                try await sourceState(source:"messages",state:readiness.state,coverage:readiness.coverage,capabilities:readiness.capabilities,reason:readiness.reason)
            }
            try wire?.send(outcome)
        } catch {
            if started {
                let nativeCode = (error as? SourceFailure).map { String(describing: $0) } ?? (error as? MessagesUIFailure)?.rawValue ?? "native_error"
                let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": .string("indeterminate"), "evidence": .object(["reason": .string("dispatch_evidence_unavailable"), "native_error_code": .string(nativeCode), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "request_hash": ticket.command["request_hash"]])])])
                try? await guardLog.append(outcome); try? wire?.send(outcome)
            } else { await rejectCommand(ticket.command, error: error) }
        }
    }
}
