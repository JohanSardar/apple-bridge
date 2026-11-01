import Foundation
import SQLite3

@main
struct ProbeTruthChecks {
    static func main() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let local = root.appendingPathComponent("streams/restricted/App.InFocus/local")
        let orphan = root.appendingPathComponent("streams/restricted/App.InFocus/remote/synthetic-retired-peer")
        for directory in [local, orphan, root.appendingPathComponent("sync")] {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        let segment = try Data(contentsOf: URL(fileURLWithPath: "lab/fixtures/usage/synthetic.segb"))
        try segment.write(to: local.appendingPathComponent("1"))
        try segment.write(to: orphan.appendingPathComponent("1"))
        var database: OpaquePointer?
        guard sqlite3_open(root.appendingPathComponent("sync/sync.db").path, &database) == SQLITE_OK else { throw UsageTruthFailure.metadataOpen }
        defer { sqlite3_close(database) }
        guard sqlite3_exec(database, "CREATE TABLE DevicePeer (device_identifier TEXT, me INTEGER, platform INTEGER); INSERT INTO DevicePeer VALUES ('synthetic-local',1,3);", nil, nil, nil) == SQLITE_OK else { throw UsageTruthFailure.metadataQuery }
        let report = try UsageTruth.observe(root: root.path)
        let rows = report["rows"] as? [String: [String: Double]] ?? [:]
        guard report["device_count"] as? Int == 1, report["unmapped_device_count"] as? Int == 1, rows.count == 1, rows.values.first?["seconds_estimated"] == 60 else { throw UsageTruthFailure.metadataRead }
        guard sqlite3_exec(database, "DELETE FROM DevicePeer;", nil, nil, nil) == SQLITE_OK else { throw UsageTruthFailure.metadataQuery }
        var refused = false
        do { _ = try UsageTruth.observe(root: root.path) }
        catch UsageTruthFailure.unknownDevice { refused = true }
        guard refused else { throw UsageTruthFailure.metadataRead }
    }
}
