import Foundation
import Darwin
import AppleSession
import NativeProtocol
import SourceSupport

final class Once: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    func take() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if done { return false }
        done = true
        return true
    }
}

final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func take() { lock.lock(); value += 1; lock.unlock() }
    var count: Int { lock.lock(); defer { lock.unlock() }; return value }
}

struct FakeSource: Source {
    let name = "phone"
    let once = Once()
    let started = Counter()
    let observes: Bool
    func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete")]), capabilities: ["phone.set"])
    }
    func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        started.take()
        guard once.take() else { try await Task.sleep(for: .seconds(3600)); return }
        let rows: [JSON] = (0..<3).map { .object(["kind": .string("upsert"), "collection": .string("phone.item"), "native_key": .string("k\($0)"), "data": .object(["n": .integer(UInt64($0))]), "observed_at": .string("2026-01-02T03:04:05Z")]) }
        try await sink.emit(source: "phone", generation: "g1", observations: rows, scanID: "scan-a", scanStartedAt: "", scope: ["phone.item"], completionEvidence: "complete", blobHandles: [:], live: false)
    }
    func prepare(_ command: JSON) async throws -> Prepared {
        Prepared(nativeState: .object(["before": .integer(1)]), fingerprint: "fp", action: command["action"])
    }
    func dispatch(_ prepared: Prepared) async throws -> JSON {
        .object(["observed": .bool(observes), "native_key": .string("k1"), "disposition": .string("locally_observed")])
    }
    func resolve(_ command: String, evidence: JSON) async -> JSON { .object(["observed": .bool(false)]) }
}

enum CheckFailure: Error { case failed(String) }
func check(_ condition: Bool, _ name: String) throws {
    guard condition else { throw CheckFailure.failed(name) }
}

final class Peer: @unchecked Sendable {
    let wire: NativeWire
    private var pending: [JSON] = []
    init(wire: NativeWire) { self.wire = wire }
    func next(_ kind: String) async throws -> JSON {
        if let index = pending.firstIndex(where: { $0["t"].string == kind }) { return pending.remove(at: index) }
        while true {
            let wire = self.wire
            let frame = try await Task.detached { try wire.receive() }.value
            if frame["t"].string == kind { return frame }
            pending.append(frame)
        }
    }
}

@main
struct SessionChecks {
    @MainActor
    static func main() async throws {
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--link-fd", let descriptor = Int32(CommandLine.arguments[2]) {
            try await live(descriptor)
            return
        }
        try await ownership()
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-session-checks-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try Data("phone".utf8).write(to: home.appendingPathComponent("lab-capture-source"))
        let session = try AppleSession(socket: home.appendingPathComponent("none.sock").path, secretFile: home.appendingPathComponent("none").path, home: home, gatewayPID: getpid(), commandsEnabled: true)
        try await session.register(FakeSource(observes: true))
        var pair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw CheckFailure.failed("socketpair") }
        let helperDescriptor = pair[0]
        let peer = Peer(wire: try NativeWire(descriptor: pair[1]))
        let running = Task { try? await session.runNative(connect: { (helperDescriptor, {}) }) }
        try peer.wire.send(.object(["t": .string("Hello"), "protocol": .string("native/1"), "version": .integer(1), "owner_fence": .integer(7)]))
        try check(try await peer.next("Welcome")["protocol"] == .string("native/1"), "welcome")
        try peer.wire.send(.object(["t": .string("Session"), "checkpoints": .object(["phone": .object(["generation": .string("g1"), "checkpoint": .string("c0")])])]))
        let ready = try await peer.next("Ready")
        try check(ready["capabilities"]["phone"].array.contains(.string("phone.set")) && ready["coverage"]["phone"] != .null, "ready lists the fake source")
        let state = try await peer.next("SourceState")
        try check(!state["source"].string.isEmpty && state["coverage"].object.count == 1, "source state shape")

        let scan = try await peer.next("Scan")
        try check(scan["source"] == .string("phone") && scan["scan_id"] == .string("scan-a") && scan["observations"].array.count == 3 && scan["scope"].array.count == 1 && scan["completion_evidence"] == .string("complete") && scan["live"] == .bool(false), "scan carries the scan fields")
        try check(scan["checkpoint"] == .null && scan["predecessor"] == .null, "swift sends no checkpoint or predecessor")
        try peer.wire.send(.object(["t": .string("ScanCommitted"), "id": scan["id"], "source": .string("phone"), "checkpoint": .string("ses-1")]))

        try peer.wire.send(.object(["t": .string("Query"), "id": .string("q1"), "op": .string("helper.version"), "args": .object([:]), "deadline_ms": .integer(1000)]))
        let result = try await peer.next("QueryResult")
        try check(result["id"] == .string("q1") && result["result_json"]["helper_build"] == .string("m1-session"), "query answered on the link")
        try peer.wire.send(.object(["t": .string("Query"), "id": .string("q2"), "op": .string("no.such"), "args": .object([:]), "deadline_ms": .integer(1000)]))
        let failure = try await peer.next("QueryError")
        try check(failure["id"] == .string("q2") && failure["code"] == .string("unsupported"), "unknown query is unsupported")

        let action: JSON = .object(["op": .string("phone.set"), "native_key": .string("k1"), "args": .object([:])])
        try peer.wire.send(.object(["t": .string("Prepare"), "command_id": .string("c1"), "principal": .string("token:abcdef"), "request_hash": .string("h1"), "origin": .string(""), "expires_at": .string(timestamp(Date().addingTimeInterval(600))), "action": action]))
        let prepared = try await peer.next("Prepared")
        try check(prepared["command_id"] == .string("c1") && prepared["fingerprint"] == .string("fp") && prepared["route"] == .string("eventkit") && prepared["native_state"]["before"] == .integer(1) && prepared["ticket"] == .null, "prepared carries the source's state and no ticket")
        try peer.wire.send(.object(["t": .string("Dispatch"), "command_id": .string("c1"), "request_hash": .string("h1"), "action": action, "native_state": prepared["native_state"], "fingerprint": .string("fp")]))
        let outcome = try await peer.next("Outcome")
        try check(outcome["disposition"] == .string("locally_observed") && outcome["evidence"]["fingerprint"] == .string("h1") && outcome["evidence"]["native_key"] == .string("k1") && outcome["evidence"]["prepared"]["action"]["op"] == .string("phone.set") && outcome["evidence"]["capture_checkpoint"] == .null, "outcome evidence is unbound")

        try peer.wire.send(.object(["t": .string("Resolve"), "command_id": .string("c1"), "saved": .object(["t": .string("Outcome"), "command_id": .string("c1"), "disposition": .string("indeterminate"), "evidence": .object(["prepared": .object(["action": action, "request_hash": .string("h1")])])])]))
        let resolved = try await peer.next("Outcome")
        try check(resolved["disposition"] == .string("indeterminate"), "unobserved resolve stays indeterminate")

        try peer.wire.send(.object(["t": .string("Prepare"), "command_id": .string("c2"), "principal": .string("token:abcdef"), "request_hash": .string("h2"), "origin": .string(""), "expires_at": .string(timestamp(Date().addingTimeInterval(-5))), "action": action]))
        let expired = try await peer.next("Rejected")
        try check(expired["command_id"] == .string("c2") && expired["code"] == .string("invalid"), "an expired command is rejected")

        try peer.wire.send(.object(["t": .string("Bogus")]))
        running.cancel()
        shutdown(pair[1], SHUT_RDWR)
        shutdown(helperDescriptor, SHUT_RDWR)
        _ = await running.value
        print("session checks passed")
    }

    @MainActor
    static func ownership() async throws {
        for flipped in [true, false] {
            let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-session-owner-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: home.appendingPathComponent("session"), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try Data("phone".utf8).write(to: home.appendingPathComponent("lab-capture-source"))
            if flipped { try Data("{\"sources\":{\"phone\":{\"root\":\"/nonexistent\"}}}".utf8).write(to: home.appendingPathComponent("session/rust-capture.json")) }
            let session = try AppleSession(socket: home.appendingPathComponent("none.sock").path, secretFile: home.appendingPathComponent("none").path, home: home, gatewayPID: getpid(), commandsEnabled: true)
            let source = FakeSource(observes: true)
            try await session.register(source)
            var pair: [Int32] = [-1, -1]
            guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw CheckFailure.failed("socketpair") }
            let helperDescriptor = pair[0]
            let peer = Peer(wire: try NativeWire(descriptor: pair[1]))
            let running = Task { try? await session.runNative(connect: { (helperDescriptor, {}) }) }
            try peer.wire.send(.object(["t": .string("Hello"), "protocol": .string("native/1"), "version": .integer(1), "owner_fence": .integer(7)]))
            _ = try await peer.next("Welcome")
            try peer.wire.send(.object(["t": .string("Session"), "checkpoints": .object([:])]))
            let ready = try await peer.next("Ready")
            try check(ready["capabilities"]["phone"].array.contains(.string("phone.set")), "a rust-owned source still serves its commands")
            try await Task.sleep(for: .seconds(2))
            try check(flipped ? source.started.count == 0 : source.started.count == 1, flipped ? "swift capture must not start for a rust-owned source" : "swift capture starts when the switch is off")
            running.cancel()
            shutdown(pair[1], SHUT_RDWR)
            shutdown(helperDescriptor, SHUT_RDWR)
            _ = await running.value
        }
        print("ownership checks passed")
    }

    @MainActor
    static func live(_ descriptor: Int32) async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-session-live-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try Data("phone".utf8).write(to: home.appendingPathComponent("lab-capture-source"))
        let session = try AppleSession(socket: home.appendingPathComponent("none.sock").path, secretFile: home.appendingPathComponent("none").path, home: home, gatewayPID: getpid(), commandsEnabled: true)
        try await session.register(FakeSource(observes: false))
        let running = Task { try? await session.runNative(connect: { (descriptor, {}) }) }
        try await Task.sleep(for: .seconds(45))
        running.cancel()
        shutdown(descriptor, SHUT_RDWR)
        _ = await running.value
    }
}
