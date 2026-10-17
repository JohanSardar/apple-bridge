import Foundation
import Darwin
import SourceSupport
import AgendaGate
import RemindersSource

@globalActor
public actor UIActor {
    public static let shared = UIActor()
}

public actor AppleSession: ObservationSink {
    private struct Pending {
        let source: String
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
    }
    private struct Ticket {
        let command: JSON
        let prepared: Prepared
        let expiry: ContinuousClock.Instant
        let fence: UInt64
    }
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
    private var commandActive = false
    private var additionalSources: [String: any Source] = [:]
    private var blobReader: (@Sendable (String, Int64, Int) async throws -> JSON)?
    private var captureTasks: [Task<Void, Never>] = []
    private var frameTasks: [UUID: Task<Void, Never>] = [:]
    private let agenda: AgendaGate
    private let source: RemindersSource
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
        let agenda = AgendaGate()
        self.agenda = agenda
        source = try RemindersSource(gate: agenda, home: home, commandsEnabled: true)
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
                var coverage: [String: JSON] = ["reminders": readiness.coverage, "calendar": .object(["fields": .string("partial"), "missing": .array([.string("calendar source pending")])])]
                var capabilities: [String: JSON] = ["reminders": .array(readiness.capabilities.map(JSON.string)), "calendar": .array([])]
                for (name, source) in additionalSources {
                    let state = await source.open()
                    coverage[name] = state.coverage
                    capabilities[name] = .array(state.capabilities.map(JSON.string))
                }
                try connected.send(.object(["t": .string("Ready"), "protocol": .string("ipc/1"), "coverage": .object(coverage), "capabilities": .object(capabilities), "helper_build": .string("m1-session"), "os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString)]))
                try await publishReadiness()
                for outcome in await guardLog.outcomes.values where outcome["disposition"].string == "indeterminate" { try connected.send(outcome) }
                captureTasks.append(Task { await self.capture() })
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
        for pending in live + bulk { pending.completion.resume(throwing: SourceFailure.unavailable) }
        for flight in flights.values { flight.pending.completion.resume(throwing: SourceFailure.unavailable) }
        live = []; bulk = []; flights = [:]; tickets = [:]
    }

    public func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        guard let wire else { throw SourceFailure.unavailable }
        try wire.send(.object(["t": .string("SourceState"), "source": .string(source), "state": .string(state), "coverage": .object([source: coverage]), "capabilities": .object([source: .array(capabilities.map(JSON.string))]), "reason": .string(reason)]))
    }

    private func publishReadiness() async throws {
        let readiness = await source.open()
        if readiness.state != permissionStates["reminders"] {
            permissionStates["reminders"] = readiness.state
            try await sourceState(source: "reminders", state: readiness.state, coverage: readiness.coverage, capabilities: readiness.capabilities, reason: readiness.reason)
        }
        let calendarAuthorized = await agenda.authorized(.event)
        let calendarState = calendarAuthorized ? "unavailable" : "permission_missing"
        if calendarState != permissionStates["calendar"] {
            permissionStates["calendar"] = calendarState
            let reason = calendarAuthorized ? "calendar_source_pending" : "calendar_full_access_required"
            try await sourceState(source: "calendar", state: calendarState, coverage: .object(["fields": .string("partial"), "missing": .array([.string(reason)])]), capabilities: [], reason: reason)
        }
    }

    private func capture() async {
        while !Task.isCancelled && wire != nil {
            do {
                try await publishReadiness()
                let readiness = await source.open()
                if readiness.state == "ready" { try await source.capture(checkpoints["reminders"]?["refresh_blobs"].bool == true ? .null : checkpoints["reminders"] ?? .null, into: self) }
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
                try await source.capture(checkpoints[source.name] ?? .null, into: self)
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
            case "Dispatch": await dispatch(frame)
            case "Resolve": await resolve(frame)
            case "BlobRead": await readBlob(frame)
            default: break
            }
            frameTasks.removeValue(forKey: id)
        }
    }

    private func readBlob(_ frame: JSON) async {
        guard let connection = wire else { return }
        do {
            guard let blobReader, !frame["handle"].string.isEmpty, frame["offset"].number >= 0, frame["offset"].number < Double(Int64.max), frame["length"].number > 0, frame["length"].number <= 131072, frame["length"].number.rounded(.down) == frame["length"].number, frame["offset"].number.rounded(.down) == frame["offset"].number else { throw SourceFailure.invalid }
            let response = try await blobReader(frame["handle"].string, Int64(frame["offset"].number), Int(frame["length"].number))
            guard wire === connection, !Task.isCancelled else { return }
            try connection.send(response)
        } catch {
            guard wire === connection, !Task.isCancelled else { return }
            try? connection.send(.object(["t": .string("BlobUnavailable"), "handle": frame["handle"], "reason": .string("unavailable")]))
        }
    }

    public func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard wire != nil else { throw SourceFailure.unavailable }
        let chunks = try split(observations)
        for chunk in chunks {
            try await withCheckedThrowingContinuation { continuation in
                let pending = Pending(source: source, generation: generation, scanID: scanID, completionEvidence: completionEvidence, blobHandles: blobHandles, observations: chunk, started: scanStartedAt, scope: scope, completion: continuation)
                if live { self.live.append(pending) } else { bulk.append(pending) }
                do { try drain() } catch { disconnect() }
            }
        }
    }

    private func split(_ observations: [JSON]) throws -> [[JSON]] {
        var chunks: [[JSON]] = [], current: [JSON] = []
        var bytes = 0
        let ordered = observations.filter { $0["kind"].string != "absent" } + observations.filter { $0["kind"].string == "absent" }
        for observation in ordered {
            let count = try JSONEncoder().encode(observation).count + 1
            guard count < 240000 else { throw SourceFailure.invalid }
            if bytes + count > 240000 { chunks.append(current); current = []; bytes = 0 }
            current.append(observation); bytes += count
        }
        if !current.isEmpty || chunks.isEmpty { chunks.append(current) }
        return chunks
    }

    private func drain() throws {
        while !live.isEmpty || !bulk.isEmpty {
            let liveIndex = live.firstIndex { flights[$0.source] == nil }
            let bulkIndex = bulk.firstIndex { flights[$0.source] == nil }
            guard let index = liveIndex ?? bulkIndex else { return }
            let prioritized = liveIndex != nil
            let pending = prioritized ? live[index] : bulk[index]
            let checkpoint = UUID().uuidString
            let resume = checkpoints[pending.source] ?? .null
            var capture: [String: JSON] = ["source": .string(pending.source), "generation": .string(pending.generation), "predecessor": .string(resume["generation"].string == pending.generation ? resume["checkpoint"].string : ""), "checkpoint": .string(checkpoint), "observations": .array(pending.observations)]
            if !pending.scanID.isEmpty {
                capture["scan_id"] = .string(pending.scanID); capture["scope"] = .array(pending.scope.map(JSON.string)); capture["scan_started_at"] = .string(pending.started.isEmpty ? timestamp() : pending.started); capture["completion_evidence"] = .string(pending.completionEvidence)
            }
            capture["blob_handles"] = .object(pending.blobHandles.mapValues(JSON.string))
            capture["t"] = .string("Capture")
            let frame = JSON.object(capture)
            let size = try PeerWire.encode(frame).count
            guard size <= credit else { return }
            try wire?.send(frame)
            credit -= size
            flights[pending.source] = Flight(pending: pending, checkpoint: checkpoint, bytes: size)
            if prioritized { live.remove(at: index) } else { bulk.remove(at: index) }
        }
    }

    private func handle(_ frame: JSON) async throws {
        switch frame["t"].string {
        case "Ping": try wire?.send(.object(["t": .string("Pong")]))
        case "Pong": break
        case "Committed":
            let source = frame["source"].string
            guard let flight = flights[source], frame["checkpoint"].string == flight.checkpoint, frame["credit"].unsigned == UInt64(flight.bytes) else { throw SourceFailure.invalid }
            try await pause("capture", observations: flight.pending.observations)
            flights.removeValue(forKey: source)
            credit += flight.bytes
            checkpoints[source] = .object(["generation": .string(flight.pending.generation), "checkpoint": .string(flight.checkpoint)])
            if labEnabled {
                let marker = home.appendingPathComponent("lab-committed-" + source)
                try Data(flight.checkpoint.utf8).write(to: marker, options: .atomic)
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: marker.path)
            }
            for observation in flight.pending.observations {
                observedCheckpoints[observation["collection"].string + "/" + observation["native_key"].string] = flight.checkpoint
            }
            flight.pending.completion.resume()
            try drain()
        case "Reset": throw SourceFailure.conflict
        case "Prepare": await prepare(frame)
        case "Dispatch": launchFrame(frame)
        case "Resolve": launchFrame(frame)
        case "BlobRead":
            launchFrame(frame)
        case "Close": throw SourceFailure.unavailable
        default: throw SourceFailure.invalid
        }
    }

    private func observedEvidence(_ evidence: JSON, fingerprint: JSON) async throws -> JSON {
        try Task.checkCancellation()
        try await source.reconcile(into: self)
        if evidence["absent"].bool { try await source.confirmAbsence(evidence["native_key"].string, into: self) }
        try Task.checkCancellation()
        let key = evidence["native_key"].string
        guard let checkpoint = observedCheckpoints["reminders.reminder/" + key] ?? observedCheckpoints["reminders.list/" + key] else { throw SourceFailure.unavailable }
        var result = evidence.object
        result["fingerprint"] = fingerprint
        result["capture_checkpoint"] = .string(checkpoint)
        return .object(result)
    }

    private func resolve(_ frame: JSON) async {
        let id = frame["command_id"].string
        do {
            guard var outcome = await guardLog.outcomes[id] else { try reject(frame, code: "not_found"); return }
            if ["indeterminate", "locally_observed"].contains(outcome["disposition"].string) {
                let evidence = await source.resolve(id, evidence: outcome["evidence"])
                if evidence["observed"].bool {
                    let committed = try await observedEvidence(evidence, fingerprint: outcome["evidence"]["prepared"]["request_hash"])
                    outcome = .object(["t": .string("Outcome"), "command_id": .string(id), "disposition": .string("locally_observed"), "evidence": committed])
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
        guard code == "conflict", !key.isEmpty else { try? reject(frame, code: code); return }
        do {
            try await source.reconcile(into: self)
            guard let checkpoint = observedCheckpoints["reminders.reminder/" + key] else { throw SourceFailure.unavailable }
            let evidence = JSON.object(["native_key": .string(key), "capture_checkpoint": .string(checkpoint)])
            try wire?.send(.object(["t": .string("Rejected"), "command_id": frame["command_id"], "disposition": .string("rejected_before_dispatch"), "error": .object(["code": .string(code), "message": .string(code)]), "evidence": evidence]))
        } catch { try? reject(frame, code: "unavailable") }
    }

    private func prepare(_ frame: JSON) async {
        do {
            guard !frame["command_id"].string.isEmpty, !frame["principal"].string.isEmpty, !frame["request_hash"].string.isEmpty, let expires = parseTimestamp(frame["expires_at"].string), expires > Date() else { throw SourceFailure.invalid }
            tickets = tickets.filter { ContinuousClock.now < $0.value.expiry }
            guard !commandActive, tickets.isEmpty, !(await guardLog.contains(frame["command_id"].string)) else { throw SourceFailure.conflict }
            let prepared = try await source.prepare(frame)
            let ticket = UUID().uuidString
            tickets[ticket] = Ticket(command: frame, prepared: prepared, expiry: ContinuousClock.now.advanced(by: .milliseconds(Int64(min(30, expires.timeIntervalSinceNow) * 1000))), fence: fence)
            try wire?.send(.object(["t": .string("Prepared"), "command_id": frame["command_id"], "ticket": .string(ticket), "native_state": prepared.nativeState, "route": .string("eventkit"), "fingerprint": frame["request_hash"], "candidate_ids": .array([])]))
        } catch { await rejectCommand(frame, error: error) }
    }

    private func dispatch(_ frame: JSON) async {
        guard let ticket = tickets.removeValue(forKey: frame["ticket"].string) else { try? reject(frame, code: "conflict"); return }
        let id = ticket.command["command_id"]
        guard !commandActive else { try? reject(ticket.command, code: "conflict"); return }
        commandActive = true
        defer { commandActive = false }
        var started = false
        do {
            guard frame["command_id"] == id, frame["owner_fence"].unsigned == fence, ticket.fence == fence, ContinuousClock.now < ticket.expiry else { throw SourceFailure.expired }
            try await source.validate(ticket.prepared)
            try Task.checkCancellation()
            guard ticket.fence == fence, ContinuousClock.now < ticket.expiry, wire != nil else { throw SourceFailure.expired }
            try await guardLog.append(.object(["t": .string("DispatchStarted"), "command_id": id, "ticket": frame["ticket"], "started_at": .string(timestamp()), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "fingerprint": .string(ticket.prepared.fingerprint), "request_hash": ticket.command["request_hash"]])]))
            started = true
            try await pause("dispatch")
            try Task.checkCancellation()
            guard ticket.fence == fence, ContinuousClock.now < ticket.expiry, wire != nil else { throw SourceFailure.conflict }
            let refetched = try await source.dispatch(ticket.prepared)
            var retained = refetched.object
            retained["prepared"] = .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "request_hash": ticket.command["request_hash"]])
            let evidence = try await observedEvidence(.object(retained), fingerprint: ticket.command["request_hash"])
            let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": .string("locally_observed"), "evidence": evidence])
            try await guardLog.append(outcome)
            try wire?.send(outcome)
        } catch {
            if started {
                let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": .string("indeterminate"), "evidence": .object(["reason": .string("dispatch_evidence_unavailable"), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "request_hash": ticket.command["request_hash"]])])])
                try? await guardLog.append(outcome); try? wire?.send(outcome)
            } else { await rejectCommand(ticket.command, error: error) }
        }
    }
}
