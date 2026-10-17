import Foundation
import SQLite3
import Decoders
import SourceSupport

struct UsageStore {
    let home: URL
    static let uncertainty = "Focus transitions only; sessions capped at 4h; open sessions omitted; Mac lock focus excluded and attributed as clipped time; historical sleep gaps and settings parity unproven; remote batches may lag"
    private func build() throws -> String {
        let process = Process(), output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/sysctl"); process.arguments = ["-n", "kern.osversion"]; process.standardOutput = output; process.standardError = FileHandle.nullDevice
        try process.run()
        let bytes = output.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw SourceFailure.unavailable }
        return String(data: bytes, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }
    func read() throws -> [JSON] {
        let biome = home.appendingPathComponent("Library/Biome")
        var database: OpaquePointer?
        guard sqlite3_open_v2(biome.appendingPathComponent("sync/sync.db").path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { if database != nil { sqlite3_close(database) }; throw SourceFailure.permissionMissing }
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT device_identifier,name,platform,model,me FROM DevicePeer ORDER BY device_identifier", -1, &statement, nil) == SQLITE_OK else { throw SourceFailure.unavailable }
        defer { sqlite3_finalize(statement) }
        let root = biome.appendingPathComponent("streams/restricted/App.InFocus")
        let localBuild = try build()
        var observations: [JSON] = []
        func emit(_ collection: String, _ key: String, _ data: [String: JSON]) {
            observations.append(.object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "data": .object(data), "observed_at": .string(timestamp())]))
        }
        while true {
            let step = sqlite3_step(statement)
            if step == SQLITE_DONE { return observations }
            guard step == SQLITE_ROW else { throw SourceFailure.unavailable }
            func text(_ column: Int32) -> String { sqlite3_column_text(statement, column).map { String(cString: $0) } ?? "" }
            let key = text(0), name = text(1), code = sqlite3_column_int(statement, 2), model = text(3).lowercased(), local = sqlite3_column_int(statement, 4) != 0
            guard !key.isEmpty else { throw SourceFailure.corrupt }
            let platform = code == 3 ? "mac" : model.contains("ipad") || model.hasPrefix("j") ? "ipad" : "iphone"
            var folder = root.appendingPathComponent(local ? "local" : "remote/" + key)
            if !local, !FileManager.default.fileExists(atPath: folder.path), FileManager.default.fileExists(atPath: root.appendingPathComponent("remote").path) {
                let candidates = try FileManager.default.contentsOfDirectory(at: root.appendingPathComponent("remote"), includingPropertiesForKeys: nil)
                if let found = candidates.first(where: { $0.lastPathComponent.lowercased() == key.lowercased() }) { folder = found }
            }
            var events: [FocusEvent] = []
            func visit(_ directory: URL) throws {
                for item in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isDirectoryKey]) {
                    if item.lastPathComponent.contains("tombstone") { continue }
                    if try item.resourceValues(forKeys: [.isDirectoryKey]).isDirectory == true { try visit(item) }
                    else if !item.lastPathComponent.isEmpty && item.lastPathComponent.allSatisfy({ $0.isNumber }) {
                        do { events += InFocus.decode(try Data(contentsOf: item, options: .mappedIfSafe)) }
                        catch { if FileManager.default.fileExists(atPath: item.path) { throw error } }
                    }
                }
            }
            if FileManager.default.fileExists(atPath: folder.path) { try visit(folder) }
            let latest = events.map(\.at).max().map { timestamp(Date(timeIntervalSince1970: $0)) } ?? ""
            emit("usage.device", key, ["name": .string(name), "platform": .string(platform), "build": .string(local ? localBuild : ""), "is_local": .bool(local), "latest_event_at": .string(latest)])
            for day in FocusUsage.aggregate(events, mac: platform == "mac") {
                let data: [String: JSON] = ["device_id": .object(["$ref": .object(["collection": .string("usage.device"), "native_key": .string(key)])]), "day": .string(day.day), "time_zone": .string(TimeZone.current.identifier), "bundle_id": .string(day.bundle), "seconds_estimated": .number((day.seconds * 1000).rounded() / 1000), "sessions": .number(Double(day.sessions)), "clipped_seconds": .number((day.clipped * 1000).rounded() / 1000), "provenance": .string("biome.app_infocus"), "decoder_version": .string(InFocus.version), "latest_event_at": .string(timestamp(Date(timeIntervalSince1970: day.latest))), "uncertainty": .string(Self.uncertainty)]
                emit("usage.app_day", key + "/" + day.day + "/" + day.bundle, data)
            }
        }
    }
}
