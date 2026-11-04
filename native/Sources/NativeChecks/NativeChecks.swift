import Foundation
import Darwin
import NativeProtocol
import SourceSupport

actor FakeNative: NativeAdapter {
    var reads = 0
    func revalidate(_ operation: String, identities: [JSON]) async throws -> [String] {
        if identities.contains(where: { $0["native_key"] == .string("short") }) { return [] }
        return identities.map { identity in
            switch identity["native_key"].string {
            case "gone": return RevalidationState.missing
            case "old": return RevalidationState.outsideScope
            default: return RevalidationState.present
            }
        }
    }
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        reads += 1
        if args["slow"] == .bool(true) { try await Task.sleep(for: .milliseconds(400)) }
        return NativeFacts(items: (0..<5).map { .object(["native_key": .string("fixture-\($0)"), "data": .object(["title": .string("synthetic-\($0)")])]) }, scope: .string(operation), generation: "fixture-1", complete: args["partial"] != .bool(true))
    }
}

struct FixtureNative: NativeAdapter {
    let fixtures: JSON
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        NativeFacts(items: fixtures[operation].array, scope: .string(operation), generation: "scenario-fixtures", complete: true)
    }
}

enum CheckFailure: Error { case failed(String) }
func check(_ condition: Bool, _ name: String) throws {
    guard condition else { throw CheckFailure.failed(name) }
}

@main
struct NativeChecks {
    static func main() async throws {
        let fake = FakeNative()
        let server = NativeServer(adapter: fake)
        let hello: JSON = .object(["t": .string("Hello"), "protocol": .string("native/1"), "version": .integer(1), "owner_fence": .integer(7)])
        try check(await server.handle(hello)["t"] == .string("Welcome"), "handshake")
        try check(await server.handle(hello)["t"] == .string("Error"), "duplicate hello")
        var request: [String: JSON] = ["t": .string("Facts"), "id": .string("facts"), "op": .string("agenda.events"), "args": .object([:]), "snapshot": .null, "cursor": .null, "limit": .integer(2)]
        var page = await server.handle(.object(request))
        try check(page["items"].array.count == 2 && !page["complete"].bool, "first page")
        let snapshot = page["snapshot"]
        request["snapshot"] = snapshot; request["cursor"] = page["cursor"]
        page = await server.handle(.object(request))
        try check(page["items"].array.count == 2 && !page["complete"].bool && page["snapshot"] == snapshot, "pinned page")
        request["op"] = .string("contacts.contacts")
        try check(await server.handle(.object(request))["code"] == .string("invalid"), "scope binding")
        request["op"] = .string("agenda.events")
        request["cursor"] = page["cursor"]
        page = await server.handle(.object(request))
        try check(page["items"].array.count == 1 && page["complete"].bool && page["cursor"] == .null, "terminal page")
        try check(await fake.reads == 1, "single snapshot fetch")
        request["op"] = .string("contacts.contacts")
        try check(await server.handle(.object(request))["code"] == .string("expired"), "terminal snapshot released")
        var closed: [String: JSON] = ["t": .string("Facts"), "id": .string("ubiquity"), "op": .string("files.ubiquity"), "args": .object(["paths": .array([.string("/a")])]), "snapshot": .null, "cursor": .null, "limit": .integer(100)]
        let ubiquity = await server.handle(.object(closed))
        try check(ubiquity["t"] == .string("FactPage") && ubiquity["op"] == .string("files.ubiquity") && ubiquity["complete"] == .bool(true), "files.ubiquity is a fact operation")
        closed["id"] = .string("mac"); closed["op"] = .string("mac.state"); closed["args"] = .object([:])
        try check(await server.handle(.object(closed))["op"] == .string("mac.state"), "mac.state is a fact operation")
        closed["id"] = .string("unknown"); closed["op"] = .string("files.search")
        try check(await server.handle(.object(closed))["code"] == .string("unsupported"), "a files query is not a fact")
        request["snapshot"] = .null; request["cursor"] = .null; request["limit"] = .integer(100); request["args"] = .object(["partial": .bool(true)])
        try check(await server.handle(.object(request))["complete"] == .bool(false), "partial scan")
        var pair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw CheckFailure.failed("socketpair") }
        defer { Darwin.close(pair[0]); Darwin.close(pair[1]) }
        let left = try NativeWire(descriptor: pair[0]), right = try NativeWire(descriptor: pair[1])
        try left.send(hello)
        try check(try right.receive() == hello, "wire roundtrip")
        var oversized: UInt32 = UInt32(NativeWire.maximumFrame + 1).bigEndian
        _ = withUnsafeBytes(of: &oversized) { Darwin.send(pair[0], $0.baseAddress!, $0.count, 0) }
        do { _ = try right.receive(); throw CheckFailure.failed("oversized accepted") } catch SourceFailure.invalid {}
        let duplicate = Data("{\"t\":\"A\",\"t\":\"B\"}".utf8)
        var duplicateLength = UInt32(duplicate.count).bigEndian
        var duplicateFrame = withUnsafeBytes(of: &duplicateLength) { Data($0) }
        duplicateFrame.append(duplicate)
        _ = duplicateFrame.withUnsafeBytes { Darwin.send(pair[0], $0.baseAddress!, $0.count, 0) }
        do { _ = try right.receive(); throw CheckFailure.failed("duplicate key accepted") } catch SourceFailure.invalid {}
        var socketPair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &socketPair) == 0 else { throw CheckFailure.failed("server socketpair") }
        let serverDescriptor = socketPair[1]
        let session = NativeServer(adapter: FakeNative())
        let serving = Task { try? await session.run(descriptor: serverDescriptor) }
        let client = try NativeWire(descriptor: socketPair[0])
        try client.send(hello)
        let welcome = try await Task.detached { try client.receive() }.value
        try check(welcome["t"] == .string("Welcome"), "socket server welcome")
        try client.send(.object(["t": .string("Facts"), "id": .string("socket-facts"), "op": .string("contacts.contacts"), "args": .object([:]), "snapshot": .null, "cursor": .null, "limit": .integer(10)]))
        let received = try await Task.detached { try client.receive() }.value
        try check(received["items"].array.count == 5 && received["complete"].bool, "socket server facts")
        shutdown(socketPair[0], SHUT_RDWR)
        Darwin.close(socketPair[0])
        _ = await serving.value
        Darwin.close(socketPair[1])
        let fixturePath = CommandLine.arguments.dropFirst().first ?? "lab/fixtures/native-1/facts.json"
        let fixtures = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: URL(fileURLWithPath: fixturePath)))
        let fixtureServer = NativeServer(adapter: FixtureNative(fixtures: fixtures))
        _ = await fixtureServer.handle(hello)
        for operation in fixtures.object.keys.sorted() {
            var rows: [JSON] = []
            var request: [String: JSON] = ["t": .string("Facts"), "id": .string(operation), "op": .string(operation), "args": .object([:]), "snapshot": .null, "cursor": .null, "limit": .integer(1)]
            while true {
                let page = await fixtureServer.handle(.object(request))
                try check(page["t"] == .string("FactPage"), "fixture page")
                rows += page["items"].array
                if page["cursor"] == .null { try check(page["complete"].bool, "fixture complete"); break }
                request["snapshot"] = page["snapshot"]; request["cursor"] = page["cursor"]
            }
            try check(rows == fixtures[operation].array, "scenario facts \(operation)")
        }
        let revalidating = NativeServer(adapter: FakeNative())
        _ = await revalidating.handle(hello)
        func ask(_ id: String, op: String = "agenda.events", collection: String = "calendar.event", keys: [String]) -> JSON {
            .object(["t": .string("Revalidate"), "id": .string(id), "op": .string(op), "args": .object(["identities": .array(keys.map { .object(["collection": .string(collection), "native_key": .string($0)]) })])])
        }
        let verdicts = await revalidating.handle(ask("verdicts", keys: ["kept", "gone", "old"]))
        try check(verdicts["t"] == .string("Revalidated") && verdicts["id"] == .string("verdicts") && verdicts["op"] == .string("agenda.events"), "revalidated frame")
        try check(verdicts["results"].array.map { $0["state"].string } == ["present", "missing", "outside_scope"] && verdicts["results"].array.map { $0["native_key"].string } == ["kept", "gone", "old"], "revalidate answers in request order")
        try check(await revalidating.handle(ask("history", op: "contacts.history", keys: ["a"]))["code"] == .string("unsupported"), "revalidate refuses an op outside the catalogue")
        try check(await revalidating.handle(ask("mismatch", collection: "calendar.calendar", keys: ["a"]))["code"] == .string("invalid"), "revalidate binds the collection to the op")
        try check(await revalidating.handle(ask("empty", keys: []))["code"] == .string("invalid"), "revalidate refuses an empty list")
        try check(await revalidating.handle(ask("large", keys: (0..<501).map { "k\($0)" }))["code"] == .string("invalid"), "revalidate refuses more than 500 identities")
        let partial = await revalidating.handle(ask("partial", keys: ["a", "short"]))
        try check(partial["t"] == .string("Error") && partial["code"] == .string("corrupt"), "a partial adapter answer is an error, never a short result")
        let slowFacts: JSON = .object(["t": .string("Facts"), "id": .string("slow"), "op": .string("agenda.events"), "args": .object(["slow": .bool(true)]), "snapshot": .null, "cursor": .null, "limit": .integer(10)])
        async let slow = revalidating.handle(slowFacts)
        try await Task.sleep(for: .milliseconds(50))
        let started = Date()
        let during = await revalidating.handle(ask("during", keys: ["kept"]))
        try check(during["t"] == .string("Revalidated") && Date().timeIntervalSince(started) < 0.3, "revalidate is answered while a page is in flight")
        try check(await slow["t"] == .string("FactPage"), "the slow page still completes")
        async let first = revalidating.handle(slowFacts)
        try await Task.sleep(for: .milliseconds(50))
        let repeated = await revalidating.handle(slowFacts)
        try check(repeated["code"] == .string("conflict"), "a request id already in flight is refused")
        _ = await first
        var pushPair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pushPair) == 0 else { throw CheckFailure.failed("push socketpair") }
        let pushServer = NativeServer(adapter: FakeNative())
        let pushing = Task { try? await pushServer.run(descriptor: pushPair[1]) }
        let pushClient = try NativeWire(descriptor: pushPair[0])
        try pushClient.send(hello)
        _ = try await Task.detached { try pushClient.receive() }.value
        await pushServer.changed(source: "calendar", generation: "native/1")
        let pushed = try await Task.detached { try pushClient.receive() }.value
        try check(pushed == .object(["t": .string("Changed"), "source": .string("calendar"), "generation": .string("native/1")]), "Changed is pushed on the same stream")
        try pushClient.send(ask("over-socket", keys: ["gone"]))
        let overSocket = try await Task.detached { try pushClient.receive() }.value
        try check(overSocket["results"].array.first?["state"] == .string("missing"), "revalidate shares the stream with the push")
        shutdown(pushPair[0], SHUT_RDWR)
        Darwin.close(pushPair[0])
        _ = await pushing.value
        Darwin.close(pushPair[1])
        try await sessionLinkChecks(hello)
        print("native/1 checks passed")
    }
}

func sessionLinkChecks(_ hello: JSON) async throws {
    var pair: [Int32] = [-1, -1]
    guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw CheckFailure.failed("session socketpair") }
    defer { Darwin.close(pair[0]); Darwin.close(pair[1]) }
    let peer = try NativeWire(descriptor: pair[1])
    let linkDescriptor = pair[0]
    let connecting = Task.detached { try NativeSessionLink(descriptor: linkDescriptor) }
    try peer.send(hello)
    let welcome = try await Task.detached { try peer.receive() }.value
    try check(welcome == .object(["t": .string("Welcome"), "protocol": .string("native/1"), "version": .integer(1)]), "session link welcome")
    try peer.send(.object(["t": .string("Session"), "checkpoints": .object(["reminders": .object(["generation": .string("g"), "checkpoint": .string("c0")])])]))
    let link = try await connecting.value
    try check(link.fence == 7 && link.checkpoints["reminders"]?["checkpoint"] == .string("c0"), "session link fence and checkpoints")
    try link.send(NativeSessionFrame.scan(id: "s1", source: "reminders", generation: "g", observations: [], scanID: "", scanStartedAt: "", scope: ["x"], completionEvidence: "complete", blobHandles: [:], live: false))
    let unscoped = try await Task.detached { try peer.receive() }.value
    try check(unscoped["id"] == .string("s1") && unscoped["scan_id"] == .null && unscoped["scope"] == .null && unscoped["completion_evidence"] == .null, "unscoped scan omits scan fields")
    try link.send(NativeSessionFrame.scan(id: "s2", source: "calendar", generation: "g", observations: [.object(["kind": .string("upsert")])], scanID: "a", scanStartedAt: "2026-01-02T03:04:05Z", scope: ["calendar.event"], completionEvidence: "complete", blobHandles: ["d": "h"], live: true))
    let scoped = try await Task.detached { try peer.receive() }.value
    try check(scoped["scan_id"] == .string("a") && scoped["scope"].array.count == 1 && scoped["live"].bool && scoped["blob_handles"]["d"] == .string("h") && scoped["completion_evidence"] == .string("complete"), "scoped scan carries its scan fields")
    try link.send(NativeSessionFrame.rejected(commandID: .string("c"), code: "conflict", message: "m", evidence: nil))
    let rejected = try await Task.detached { try peer.receive() }.value
    try check(rejected["code"] == .string("conflict") && rejected["evidence"] == .null, "rejected frame")
    link.close()
    var bad: [Int32] = [-1, -1]
    guard socketpair(AF_UNIX, SOCK_STREAM, 0, &bad) == 0 else { throw CheckFailure.failed("bad socketpair") }
    defer { Darwin.close(bad[0]); Darwin.close(bad[1]) }
    let badPeer = try NativeWire(descriptor: bad[1])
    let refusedDescriptor = bad[0]
    let refusing = Task.detached { try NativeSessionLink(descriptor: refusedDescriptor) }
    try badPeer.send(.object(["t": .string("Hello"), "protocol": .string("native/2"), "version": .integer(1), "owner_fence": .integer(7)]))
    do { _ = try await refusing.value; throw CheckFailure.failed("wrong protocol accepted") } catch SourceFailure.invalid {}
}
