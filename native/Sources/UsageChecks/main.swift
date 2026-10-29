import Foundation
import Decoders
import UsageSource
import SourceSupport

@main
struct UsageChecks {
    static func main() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let a = InFocus.decode(try Data(contentsOf: root.appendingPathComponent("synthetic.segb")))
        let b = InFocus.decode(try Data(contentsOf: root.appendingPathComponent("malformed.segb")))
        let crlf = InFocus.decode(try Data(contentsOf: root.appendingPathComponent("crlf.segb")))
        guard a.count == 2, Set(a) == Set(b), crlf.count == 2, crlf[1].time - crlf[0].time == 60 else { throw SourceFailure.corrupt }
        let zone = TimeZone(identifier: "America/New_York")!
        let start = 1800100800.0
        func e(_ offset: Double, _ starting: Bool, _ app: String = "test.synthetic.alpha") -> InFocusEvent { InFocusEvent(time: start + offset, starting: starting, bundle: app) }
        func totals(_ events: [InFocusEvent], mac: Bool = true) -> (Double, Double) {
            let rows = UsageRollup.records(events, device: "synthetic", mac: mac, zone: zone).values
            return (rows.reduce(0) { $0 + $1["seconds_estimated"].number }, rows.reduce(0) { $0 + $1["clipped_seconds"].number })
        }
        guard totals(a + a).0 == 60, totals([e(0,true), e(600,true,"loginwindow"), e(3600,false)]).0 == 600, totals([e(0,true),e(600,true,"loginwindow"),e(3600,false)]).1 == 3000 else { throw SourceFailure.corrupt }
        guard totals([e(0,true),e(20000,false)]).0 == 14400, totals([e(0,true)]).0 == 0, totals([e(0,true),e(120,true,"test.synthetic.beta"),e(180,false,"test.synthetic.beta")]).0 == 180 else { throw SourceFailure.corrupt }
        guard totals([e(0,true),e(60,true,"com.apple.ScreenSaver.Engine"),e(600,false)]).0 == 60, totals([e(0,true),e(90,true,"bridge.power.sleep"),e(600,false)]).0 == 90 else { throw SourceFailure.corrupt }
        let format = ISO8601DateFormatter()
        for pair in [("2026-03-08T04:30:00Z", "2026-03-08T08:30:00Z", 14400.0), ("2026-11-01T03:30:00Z", "2026-11-01T07:30:00Z", 14400.0)] {
            let events = [InFocusEvent(time: format.date(from: pair.0)!.timeIntervalSince1970, starting: true, bundle: "test.synthetic.alpha"), InFocusEvent(time: format.date(from: pair.1)!.timeIntervalSince1970, starting: false, bundle: "test.synthetic.alpha")]
            let rows = UsageRollup.records(events, device: "synthetic", mac: true, zone: zone)
            guard rows.count == 2, rows.values.reduce(0, { $0 + $1["seconds_estimated"].number }) == pair.2 else { throw SourceFailure.corrupt }
        }
        let biome = FileManager.default.temporaryDirectory.appendingPathComponent("usage-inputs-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: biome) }
        let local = biome.appendingPathComponent("streams/restricted/App.InFocus/local"), remote = biome.appendingPathComponent("streams/restricted/App.InFocus/remote/peer")
        try FileManager.default.createDirectory(at: local, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: biome.appendingPathComponent("sync"), withIntermediateDirectories: true)
        try Data([1]).write(to: local.appendingPathComponent("1"))
        try Data([1]).write(to: biome.appendingPathComponent("sync/sync.db"))
        guard let idle = UsageInputs(biome, power: 0, zone: zone), UsageInputs(biome, power: 0, zone: zone, previous: idle) == idle else { throw SourceFailure.corrupt }
        let watcher = UsageWatcher(root: biome)
        let changed = Task {
            for await _ in watcher.changes { return true }
            return false
        }
        let timeout = Task { try await Task.sleep(for: .seconds(3)); changed.cancel() }
        let segment = try FileHandle(forWritingTo: local.appendingPathComponent("1"))
        try segment.seekToEnd(); try segment.write(contentsOf: Data([2])); try segment.close()
        guard await changed.value else { throw SourceFailure.unavailable }
        timeout.cancel()
        guard let appended = UsageInputs(biome, power: 0, zone: zone, previous: idle), appended != idle else { throw SourceFailure.corrupt }
        try FileManager.default.createDirectory(at: remote, withIntermediateDirectories: true)
        try Data([1]).write(to: remote.appendingPathComponent("1"))
        guard let synced = UsageInputs(biome, power: 0, zone: zone, previous: appended), synced != appended, UsageInputs(biome, power: 1, zone: zone, previous: synced) != synced, UsageInputs(biome, power: 0, zone: TimeZone(identifier: "Europe/Paris")!, previous: synced) != synced else { throw SourceFailure.corrupt }
        try Data([2]).write(to: biome.appendingPathComponent("sync/sync.db"))
        guard let renamed = UsageInputs(biome, power: 0, zone: zone, previous: synced), renamed != synced, UsageInputs(biome.appendingPathComponent("absent"), power: 0, zone: zone, previous: synced) == nil else { throw SourceFailure.corrupt }
        try FileManager.default.removeItem(at: remote)
        guard let removed = UsageInputs(biome, power: 0, zone: zone, previous: renamed), removed != renamed else { throw SourceFailure.corrupt }
        let wal = biome.appendingPathComponent("sync/sync.db-wal")
        try Data([1]).write(to: wal)
        guard let createdWAL = UsageInputs(biome, power: 0, zone: zone, previous: removed), createdWAL != removed else { throw SourceFailure.corrupt }
        try FileManager.default.removeItem(at: wal)
        guard UsageInputs(biome, power: 0, zone: zone, previous: createdWAL) != createdWAL else { throw SourceFailure.corrupt }
        print("usage decoder, day math and scan inputs: pass")
    }
}
