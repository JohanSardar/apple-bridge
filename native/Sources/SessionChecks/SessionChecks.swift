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
    func status() async -> JSON? { .object(["ok": .bool(true)]) }
    func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard operation == "phone.echo" else { throw SourceFailure.unsupported }
        return .object(["echo": args["value"]])
    }
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
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-session-checks-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
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
        try check(ready["framework"]["phone"]["ok"] == .bool(true) && ready["coverage"] == .null, "ready carries raw status only")
        let state = try await peer.next("FrameworkStatus")
        try check(state["status"]["phone"]["ok"] == .bool(true), "framework status")

        try peer.wire.send(.object(["t": .string("Query"), "id": .string("q1"), "op": .string("helper.version"), "args": .object([:]), "deadline_ms": .integer(1000)]))
        let result = try await peer.next("QueryResult")
        try check(result["id"] == .string("q1") && result["result_json"]["helper_build"] == .string("m1-session"), "query answered on the link")
        try peer.wire.send(.object(["t": .string("Query"), "id": .string("q2"), "op": .string("no.such"), "args": .object([:]), "deadline_ms": .integer(1000)]))
        let failure = try await peer.next("QueryError")
        try check(failure["id"] == .string("q2") && failure["code"] == .string("unsupported"), "unknown query is unsupported")

        try peer.wire.send(.object(["t": .string("Primitive"), "id": .string("p1"), "op": .string("phone.echo"), "args": .object(["value": .integer(3)])]))
        let answer = try await peer.next("PrimitiveResult")
        try check(answer["id"] == .string("p1") && answer["result"]["echo"] == .integer(3), "primitive reaches its source")
        try peer.wire.send(.object(["t": .string("Primitive"), "id": .string("p2"), "op": .string("phone.other"), "args": .object([:])]))
        let refused = try await peer.next("PrimitiveError")
        try check(refused["id"] == .string("p2") && refused["code"] == .string("unsupported"), "unknown primitive is unsupported")
        try peer.wire.send(.object(["t": .string("Primitive"), "id": .string("p3"), "op": .string("nowhere.other"), "args": .object([:])]))
        try check(try await peer.next("PrimitiveError")["id"] == .string("p3"), "primitive for no source is refused")

        try peer.wire.send(.object(["t": .string("Bogus")]))
        running.cancel()
        shutdown(pair[1], SHUT_RDWR)
        shutdown(helperDescriptor, SHUT_RDWR)
        _ = await running.value
        print("session checks passed")
    }

    @MainActor
    static func live(_ descriptor: Int32) async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ab-session-live-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let session = try AppleSession(socket: home.appendingPathComponent("none.sock").path, secretFile: home.appendingPathComponent("none").path, home: home, gatewayPID: getpid(), commandsEnabled: true)
        try await session.register(FakeSource(observes: false))
        let running = Task { try? await session.runNative(connect: { (descriptor, {}) }) }
        try await Task.sleep(for: .seconds(45))
        running.cancel()
        shutdown(descriptor, SHUT_RDWR)
        _ = await running.value
    }
}
