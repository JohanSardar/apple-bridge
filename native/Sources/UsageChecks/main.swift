import Foundation
import Decoders
import UsageSource
import SourceSupport

@main
struct UsageChecks {
    static func main() throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let a = InFocus.decode(try Data(contentsOf: root.appendingPathComponent("synthetic.segb")))
        let b = InFocus.decode(try Data(contentsOf: root.appendingPathComponent("malformed.segb")))
        guard a.count == 2, Set(a) == Set(b) else { throw SourceFailure.corrupt }
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
        print("usage decoder and day math: pass")
    }
}
