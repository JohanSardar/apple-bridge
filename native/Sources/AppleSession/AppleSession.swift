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
    private var live: [Pending] = []
    private var bulk: [Pending] = []
    private var flights: [String: Flight] = [:]
    private var tickets: [String: Ticket] = [:]
    private var captureTasks: [Task<Void, Never>] = []
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
        source = try RemindersSource(gate: agenda, home: home, commandsEnabled: commandsEnabled)
        guardLog = try DispatchGuard(home: home)
        let pidFile = home.appendingPathComponent("helper.pid")
        try Data(String(getpid()).utf8).write(to: pidFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pidFile.path)
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
                try connected.send(.object(["t": .string("Ready"), "protocol": .string("ipc/1"), "coverage": .object(["reminders": readiness.coverage, "calendar": .object(["fields": .string("partial"), "missing": .array([.string("calendar source pending")])])]), "capabilities": .object(["reminders": .array(readiness.capabilities.map(JSON.string)), "calendar": .array([])]), "helper_build": .string("m1-session"), "os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString)]))
                try await publishReadiness()
                for outcome in await guardLog.outcomes.values where outcome["disposition"].string == "indeterminate" { try connected.send(outcome) }
                captureTasks.append(Task { await self.capture() })
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
        for task in captureTasks { task.cancel() }
        captureTasks = []
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
                if readiness.state == "ready" { try await source.capture(checkpoints["reminders"] ?? .null, into: self) }
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
            let prioritized = !live.isEmpty
            let pending = prioritized ? live[0] : bulk[0]
            if flights[pending.source] != nil { return }
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
            if prioritized { live.removeFirst() } else { bulk.removeFirst() }
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
            flight.pending.completion.resume()
            try drain()
        case "Reset": throw SourceFailure.conflict
        case "Prepare": await prepare(frame)
        case "Dispatch": await dispatch(frame)
        case "Resolve":
            let id = frame["command_id"].string
            if var outcome = await guardLog.outcomes[id] {
                if outcome["disposition"].string == "indeterminate" {
                    let evidence = await source.resolve(id, evidence: outcome["evidence"])
                    if evidence["observed"].bool {
                        outcome = .object(["t": .string("Outcome"), "command_id": .string(id), "disposition": .string("locally_observed"), "evidence": evidence])
                        try await guardLog.append(outcome)
                    }
                }
                try wire?.send(outcome)
            } else { try reject(frame, code: "not_found") }
        case "BlobRead":
            try wire?.send(.object(["t": .string("BlobUnavailable"), "handle": frame["handle"], "reason": .string("unsupported")]))
        case "Close": throw SourceFailure.unavailable
        default: throw SourceFailure.invalid
        }
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

    private func prepare(_ frame: JSON) async {
        do {
            guard !frame["command_id"].string.isEmpty, !frame["principal"].string.isEmpty, !frame["request_hash"].string.isEmpty, let expires = parseTimestamp(frame["expires_at"].string), expires > Date() else { throw SourceFailure.invalid }
            tickets = tickets.filter { ContinuousClock.now < $0.value.expiry }
            guard tickets.isEmpty, !(await guardLog.contains(frame["command_id"].string)) else { throw SourceFailure.conflict }
            let prepared = try await source.prepare(frame)
            let ticket = UUID().uuidString
            tickets[ticket] = Ticket(command: frame, prepared: prepared, expiry: ContinuousClock.now.advanced(by: .milliseconds(Int64(min(30, expires.timeIntervalSinceNow) * 1000))), fence: fence)
            try wire?.send(.object(["t": .string("Prepared"), "command_id": frame["command_id"], "ticket": .string(ticket), "native_state": prepared.nativeState, "route": .string("eventkit"), "fingerprint": .string(prepared.fingerprint), "candidate_ids": .array([])]))
        } catch { try? reject(frame, code: errorCode(error)) }
    }

    private func dispatch(_ frame: JSON) async {
        guard let ticket = tickets.removeValue(forKey: frame["ticket"].string) else { try? reject(frame, code: "conflict"); return }
        let id = ticket.command["command_id"]
        var started = false
        do {
            guard frame["owner_fence"].unsigned == fence, ticket.fence == fence, ContinuousClock.now < ticket.expiry else { throw SourceFailure.expired }
            try await source.validate(ticket.prepared)
            try await guardLog.append(.object(["t": .string("DispatchStarted"), "command_id": id, "ticket": frame["ticket"], "started_at": .string(timestamp()), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState, "fingerprint": .string(ticket.prepared.fingerprint)])]))
            started = true
            try await pause("dispatch")
            let evidence = try await source.dispatch(ticket.prepared)
            let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": .string("locally_observed"), "evidence": evidence])
            try await guardLog.append(outcome)
            try wire?.send(outcome)
        } catch {
            if started {
                let outcome = JSON.object(["t": .string("Outcome"), "command_id": id, "disposition": .string("indeterminate"), "evidence": .object(["reason": .string("dispatch_evidence_unavailable"), "prepared": .object(["action": ticket.prepared.action, "native_state": ticket.prepared.nativeState])])])
                try? await guardLog.append(outcome); try? wire?.send(outcome)
            } else { try? reject(ticket.command, code: errorCode(error)) }
        }
    }
}
