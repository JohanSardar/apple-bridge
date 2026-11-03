import Foundation
import Darwin
import NativeProtocol
import SourceSupport

actor FakeNative: NativeAdapter {
    var effects = 0
    var reads = 0
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        reads += 1
        return NativeFacts(items: (0..<5).map { .object(["native_key": .string("fixture-\($0)"), "data": .object(["title": .string("synthetic-\($0)")])]) }, scope: .array([.string(operation)]), generation: "fixture-1", complete: args["partial"] != .bool(true))
    }
    func effect(_ operation: String, args: JSON, permit: JSON) async throws -> JSON {
        effects += 1
        return .object(["observed": .bool(true)])
    }
}

struct FixtureNative: NativeAdapter {
    let fixtures: JSON
    func facts(_ operation: String, args: JSON) async throws -> NativeFacts {
        NativeFacts(items: fixtures[operation].array, scope: .string(operation), generation: "scenario-fixtures", complete: true)
    }
    func effect(_ operation: String, args: JSON, permit: JSON) async throws -> JSON { throw SourceFailure.unsupported }
}

enum CheckFailure: Error { case failed(String) }
func check(_ condition: Bool, _ name: String) throws {
    guard condition else { throw CheckFailure.failed(name) }
}

@main
struct NativeChecks {
    static func main() async throws {
        let hashArgs: JSON = .object(["z": .string("/"), "a": .integer(1)])
        try check(try NativeServer.requestHash(operation: "messages.reply", args: hashArgs) == "c8d528c1730c15621d5062c81c3c92bf6bac4a4858685dbd8404e0a7a8a794c8", "canonical hash")
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
        request["snapshot"] = .null; request["cursor"] = .null; request["limit"] = .integer(100); request["args"] = .object(["partial": .bool(true)])
        try check(await server.handle(.object(request))["complete"] == .bool(false), "partial scan")
        let args: JSON = .object(["text": .string("synthetic")])
        var effect: [String: JSON] = ["t": .string("Effect"), "id": .string("effect"), "op": .string("messages.reply"), "args": args]
        try check(await server.handle(.object(effect))["t"] == .string("Error"), "missing permit")
        let permit: [String: JSON] = ["command_id": .string("command-1"), "owner_fence": .integer(7), "ticket": .string("ticket-1"), "request_hash": .string(try NativeServer.requestHash(operation: "messages.reply", args: args)), "operation": .string("messages.reply"), "expires_at": .integer(UInt64(Date().timeIntervalSince1970) + 30), "nonce": .string(String(repeating: "a", count: 64))]
        for key in ["operation", "owner_fence", "request_hash", "expires_at"] {
            var invalid = permit
            invalid[key] = key == "owner_fence" ? .integer(8) : .string("wrong")
            effect["permit"] = .object(invalid)
            try check(await server.handle(.object(effect))["t"] == .string("Error"), "permit \(key)")
        }
        try check(await fake.effects == 0, "reject before adapter")
        effect["permit"] = .object(permit)
        try check(await server.handle(.object(effect))["t"] == .string("EffectResult"), "valid permit")
        try check(await server.handle(.object(effect))["code"] == .string("conflict"), "one shot")
        try check(await fake.effects == 1, "exactly once")
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
        let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath().appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let files = try NativeFiles(root: root)
        let bytes = Data("synthetic payload".utf8)
        let reference = try await files.write(bytes, mime: "text/plain")
        try check(try await files.read(reference) == bytes, "file roundtrip")
        var traversal = reference.object; traversal["path"] = .string("../escape")
        do { _ = try await files.read(.object(traversal)); throw CheckFailure.failed("traversal accepted") } catch SourceFailure.invalid {}
        let linked = root.appendingPathComponent("linked")
        try FileManager.default.linkItem(at: root.appendingPathComponent(reference["path"].string), to: linked)
        do { _ = try await files.read(reference); throw CheckFailure.failed("hardlink accepted") } catch SourceFailure.invalid {}
        try FileManager.default.removeItem(at: linked)
        let symlink = root.appendingPathComponent("symlink")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: root.appendingPathComponent(reference["path"].string))
        var symlinkRef = reference.object; symlinkRef["path"] = .string("symlink")
        do { _ = try await files.read(.object(symlinkRef)); throw CheckFailure.failed("symlink accepted") } catch SourceFailure.invalid {}
        try await files.release(reference)
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
        print("native/1 checks passed")
    }
}
