import Foundation
import AppKit
import SafariSource
import SourceSupport
import CryptoKit
import Darwin
import SQLite3

@main
struct Main {
    static func main() async throws {
        if CommandLine.arguments.count > 2 && CommandLine.arguments[2] == "SAF-TABS-ONLY" {
            try await tabsOnlyChecks()
            print("tabs-only ok")
            return
        }
        if CommandLine.arguments.count > 2 && ["SAF-HISTORY-REAL", "SAF-BOOKMARKS-REAL"].contains(CommandLine.arguments[2]) {
            let source = SafariSource(directory: URL(fileURLWithPath: CommandLine.arguments[1]))
            let rows: [JSON]
            if CommandLine.arguments[2] == "SAF-HISTORY-REAL" { (_, rows) = try await source.history(days: 7) }
            else { rows = try await source.bookmarks() }
            let count = CommandLine.arguments[2] == "SAF-HISTORY-REAL" ? rows.reduce(0) { $0 + Int($1["data"]["visit_count"].number) } : rows.count
            let digest = SHA256.hash(data: Data(rows.filter { $0["collection"].string == "safari.bookmark" }.map { $0["data"]["url"].string + "\0" + $0["data"]["title"].string + "\0" + $0["data"]["folder_path"].string }.sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }.joined(separator: "\n").utf8)).map { String(format: "%02x", $0) }.joined()
            print("\(count) \(digest)")
            return
        }
        let cacheURL = URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("bridge-state/safari.json")
        let source = SafariSource(directory: URL(fileURLWithPath: CommandLine.arguments[1]), cacheURL: cacheURL)
        guard await source.open().state == "ready" else { throw SourceFailure.unavailable }
        let missing = SafariSource(directory: URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("missing"))
        guard await missing.open().state == "unavailable" else { throw SourceFailure.corrupt }
        let (generation, history) = try await source.history()
        let bookmarks = try await source.bookmarks()
        let (_, cloud) = try await source.cloudTabs()
        guard history.count == 1, history[0]["data"]["visit_count"].number == 2,
              history[0]["data"]["title"].string == "Latest",
              bookmarks.count == 3,
              bookmarks.contains(where: { $0["collection"].string == "safari.bookmark" && $0["data"]["folder_path"].string == "Favorites" }),
              bookmarks.contains(where: { $0["collection"].string == "safari.reading_list" && !$0["data"]["read"].bool }),
              bookmarks.contains(where: { $0["collection"].string == "safari.reading_list" && $0["data"]["read"].bool && parseTimestamp($0["data"]["added"].string) == Date(timeIntervalSince1970: 1767225600) }),
              cloud.count == 2, cloud.contains(where: { $0["data"]["device"].string == "Synthetic" }),
              cloud.contains(where: { $0["data"]["device"].string == "unknown-device" }) else { throw SourceFailure.corrupt }
        if CommandLine.arguments.last == "SAF-IDLE" {
            let sink = CaptureSink()
            let capture = Task { try await source.capture(.null, into: sink) }
            defer { capture.cancel() }
            try await Task.sleep(for: .seconds(2))
            guard await sink.upserts == 6, await sink.failures == 0 else { throw SourceFailure.unavailable }
            let started = ProcessInfo.processInfo.systemUptime
            let before = clock()
            try await Task.sleep(for: .seconds(300))
            let after = clock()
            guard before != ~clock_t(0), after != ~clock_t(0), after >= before else { throw SourceFailure.unavailable }
            let cpu = Double(after - before) / Double(CLOCKS_PER_SEC) / (ProcessInfo.processInfo.systemUptime - started) * 100
            capture.cancel()
            _ = try? await capture.value
            guard await sink.failures == 0 else { throw SourceFailure.unavailable }
            print("Safari idle cpu \(cpu)")
            return
        }
        let captureSink = CaptureSink()
        let captureTask = Task { try await source.capture(.null, into: captureSink) }
        for _ in 0..<100 {
            if await captureSink.upserts >= 6 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await captureSink.upserts >= 6 else { throw SourceFailure.unavailable }
        let bookmarksPath = URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("Bookmarks.plist")
        try PropertyListSerialization.data(fromPropertyList: ["Children": [[String: String]]()], format: .binary, options: 0).write(to: bookmarksPath, options: .atomic)
        for _ in 0..<100 {
            if await captureSink.absences == 3 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await captureSink.absences == 3 else { throw SourceFailure.corrupt }
        try Data("invalid".utf8).write(to: bookmarksPath)
        for _ in 0..<100 {
            if await captureSink.failures > 0 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await captureSink.failures > 0, await captureSink.absences == 3 else { throw SourceFailure.corrupt }
        do { _ = try await source.history(maxRows: 0); throw SourceFailure.invalid } catch SourceFailure.unavailable {}
        let watcher = DatabaseWatcher(path: URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("History.db").path)
        let watchStarted = Date()
        let event = Task { for await _ in watcher.changes { return true }; return false }
        var db: OpaquePointer?
        try openFixtureDatabase(URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("History.db").path, &db)
        guard sqlite3_exec(db, "PRAGMA journal_mode=WAL; UPDATE history_visits SET title='Changed' WHERE id=2", nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        guard await event.value, Date().timeIntervalSince(watchStarted) < 5 else { throw SourceFailure.unavailable }
        let (_, changed) = try await source.history()
        guard changed[0]["data"]["title"].string == "Changed" else { throw SourceFailure.corrupt }
        for _ in 0..<100 {
            if await captureSink.changedHistory { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await captureSink.changedHistory else { throw SourceFailure.corrupt }
        sqlite3_busy_timeout(db, 1000)
        guard sqlite3_wal_checkpoint_v2(db, nil, SQLITE_CHECKPOINT_TRUNCATE, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        sqlite3_close(db)
        watcher.close()
        let historyPath = URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("History.db")
        let replacement = historyPath.appendingPathExtension("replacement")
        try FileManager.default.copyItem(at: historyPath, to: replacement)
        try openFixtureDatabase(replacement.path, &db)
        guard sqlite3_exec(db, "UPDATE history_items SET url='https://example.com/rebuilt' WHERE id=1", nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        sqlite3_close(db)
        try FileManager.default.removeItem(at: historyPath)
        try FileManager.default.moveItem(at: replacement, to: historyPath)
        let (nextGeneration, rebuilt) = try await source.history()
        guard nextGeneration != generation, rebuilt.count == 1 else { throw SourceFailure.corrupt }
        for _ in 0..<100 {
            if await captureSink.absences == 4 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await captureSink.absences == 4, await captureSink.replacementExcludedAbsence else { throw SourceFailure.corrupt }
        captureTask.cancel()
        _ = try? await captureTask.value
        try Data("invalid".utf8).write(to: URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("Bookmarks.plist"))
        do { _ = try await source.bookmarks(); throw SourceFailure.invalid } catch SourceFailure.invalid { throw SourceFailure.invalid } catch {}
        let cacheText = String(data: try Data(contentsOf: cacheURL), encoding: .utf8) ?? ""
        guard !["https://", "Latest", "Favorites", "Synthetic"].contains(where: { cacheText.contains($0) }),
              (try FileManager.default.attributesOfItem(atPath: cacheURL.path)[.posixPermissions] as? NSNumber)?.intValue == 0o600 else { throw SourceFailure.corrupt }
        try openFixtureDatabase(historyPath.path, &db)
        guard sqlite3_exec(db, "DELETE FROM history_visits; DELETE FROM history_items", nil, nil, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        sqlite3_close(db)
        let resumed = SafariSource(directory: URL(fileURLWithPath: CommandLine.arguments[1]), cacheURL: cacheURL)
        let resumedSink = CaptureSink()
        let resumedTask = Task { try await resumed.capture(.null, into: resumedSink) }
        for _ in 0..<100 {
            if await resumedSink.absences == 1 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        let sourceGeneration = await captureSink.generation
        guard await resumedSink.absences == 1, await resumedSink.generation == sourceGeneration else { throw SourceFailure.corrupt }
        resumedTask.cancel()
        _ = try? await resumedTask.value
        try await tabsOnlyChecks()
        print(String(data: try JSONEncoder().encode(history + bookmarks + cloud), encoding: .utf8)!)
    }
    static func tabsOnlyChecks() async throws {
        let state = TabState()
        let tabs = SafariTabs(execute: { _ in state.descriptor() })
        let source = SafariSource(directory: URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("tabs-only-missing"), tabs: tabs, tabsOnly: true)
        let sink = TabSink()
        let task = Task { try await source.capture(.null, into: sink) }
        for _ in 0..<100 {
            if await sink.frames.count == 1 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await sink.frames.count == 1, await sink.frames[0].keys.isEmpty, await sink.frames[0].scope == ["safari.tab"] else { throw SourceFailure.corrupt }
        state.set([(7, 1, "One", "https://example.com/a", true), (7, 2, "Two", "https://example.com/b", false)])
        for _ in 0..<200 {
            if await sink.frames.count == 2 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await sink.frames.count == 2, await sink.frames[1].keys == ["7:1", "7:2"], await sink.frames[1].absent.isEmpty else { throw SourceFailure.corrupt }
        state.set([(7, 1, "One", "https://example.com/a", true)])
        for _ in 0..<200 {
            if await sink.frames.count == 3 { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard await sink.frames.count == 3, await sink.frames[2].keys == ["7:1"], await sink.frames[2].absent == ["7:2"] else { throw SourceFailure.corrupt }
        guard await sink.frames.allSatisfy({ $0.scope == ["safari.tab"] && $0.source == "safari" }) else { throw SourceFailure.corrupt }
        task.cancel()
        _ = try? await task.value
    }
}

final class TabState: @unchecked Sendable {
    private let lock = NSLock()
    private var rows: [(Int32, Int32, String, String, Bool)] = []
    func set(_ value: [(Int32, Int32, String, String, Bool)]) { lock.lock(); rows = value; lock.unlock() }
    func descriptor() -> NSAppleEventDescriptor {
        lock.lock(); defer { lock.unlock() }
        let list = NSAppleEventDescriptor.list()
        for (index, row) in rows.enumerated() {
            let item = NSAppleEventDescriptor.list()
            item.insert(NSAppleEventDescriptor(int32: row.0), at: 1)
            item.insert(NSAppleEventDescriptor(int32: row.1), at: 2)
            item.insert(NSAppleEventDescriptor(string: row.2), at: 3)
            item.insert(NSAppleEventDescriptor(string: row.3), at: 4)
            item.insert(NSAppleEventDescriptor(boolean: row.4), at: 5)
            list.insert(item, at: index + 1)
        }
        return list
    }
}

actor TabSink: ObservationSink {
    struct Frame { let source: String; let scope: [String]; let keys: [String]; let absent: [String] }
    var frames: [Frame] = []
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard completionEvidence == "complete" else { throw SourceFailure.corrupt }
        frames.append(Frame(source: source, scope: scope, keys: observations.filter { $0["kind"].string == "upsert" }.map { $0["native_key"].string }, absent: observations.filter { $0["kind"].string == "absent" }.map { $0["native_key"].string }))
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.unsupported }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws { throw SourceFailure.corrupt }
}

actor CaptureSink: ObservationSink {
    var upserts = 0
    var absences = 0
    var failures = 0
    var generation = ""
    var changedHistory = false
    var replacementExcludedAbsence = false
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard completionEvidence == "complete", !scanStartedAt.isEmpty, !scope.isEmpty else { throw SourceFailure.corrupt }
        guard self.generation.isEmpty || self.generation == generation else { throw SourceFailure.corrupt }
        self.generation = generation
        if observations.contains(where: { $0["collection"].string == "safari.history" && $0["data"]["title"].string == "Changed" }) { changedHistory = true }
        if observations.contains(where: { $0["collection"].string == "safari.history" && $0["data"]["url"].string == "https://example.com/rebuilt" }) {
            replacementExcludedAbsence = !observations.contains(where: { $0["kind"].string == "absent" })
        }
        upserts += observations.filter { $0["kind"].string == "upsert" }.count
        absences += observations.filter { $0["kind"].string == "absent" && $0["revalidated"].bool }.count
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.unsupported }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws { failures += 1 }
}

private func openFixtureDatabase(_ path: String, _ database: inout OpaquePointer?) throws {
    guard sqlite3_open(path, &database) == SQLITE_OK else { throw SourceFailure.unavailable }
    var enabled: Int32 = 1
    guard sqlite3_file_control(database, "main", SQLITE_FCNTL_PERSIST_WAL, &enabled) == SQLITE_OK else { throw SourceFailure.unavailable }
}
