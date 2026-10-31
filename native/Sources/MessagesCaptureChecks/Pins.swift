import Foundation
import CryptoKit
import MessagesSource
import SourceSupport

enum PinsCaptureChecks {
    static func run() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-pins-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let path = root.appendingPathComponent("chat.db").path
        let pins = root.appendingPathComponent("pins.plist")
        defer {
            let digest = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
            let baseline = NSHomeDirectory() + "/Library/Application Support/AppleBridge/messages/" + digest + ".sqlite"
            for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: baseline + suffix) }
            try? FileManager.default.removeItem(at: root)
        }
        func write(_ identifiers: [String]) throws {
            try PropertyListSerialization.data(fromPropertyList: ["pD": ["pP": identifiers]], format: .binary, options: 0).write(to: pins, options: .atomic)
        }
        let db = try SyntheticDatabase(path)
        try db.execute("PRAGMA journal_mode=WAL; CREATE TABLE message(guid TEXT); CREATE TABLE chat(guid TEXT,chat_identifier TEXT); INSERT INTO chat VALUES('synthetic-a','pin-a'),('synthetic-b','pin-b');")
        try write([])
        let source = Messages(path: path, watcherEventsEnabled: false, pinsURL: pins)
        let sink = CaptureSink()
        let task = Task { try await source.capture(.null, into: sink) }
        defer { task.cancel() }
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait { await sink.completed() }, "pin baseline")
        try write(["pin-b", "pin-a"])
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait { let a = await sink.record("synthetic-a"); let b = await sink.record("synthetic-b"); return a["pinned_index"] == .number(1) && b["pinned_index"] == .number(0) }, "pin preferences captured without database change")
        try write(["pin-a"])
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait { let a = await sink.record("synthetic-a"); let b = await sink.record("synthetic-b"); return a["pinned_index"] == .number(0) && b["pinned_index"] == .null }, "unpin and reordered index captured")
        task.cancel()
        _ = try? await task.value
        print("all Messages pin capture checks passed")
    }
}
