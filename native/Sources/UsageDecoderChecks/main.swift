import Foundation
import Decoders

@main
struct UsageDecoderChecks {
    static func main() throws {
        let root = CommandLine.arguments[1]
        let bytes = try Data(contentsOf: URL(fileURLWithPath: root + "/scrubbed.segb"))
        let events = InFocus.decode(bytes)
        guard events.count == 6 else { throw CheckFailure.records }
        let totals = FocusUsage.aggregate(events + events, mac: true, timeZone: TimeZone(secondsFromGMT: 0)!)
        guard totals.count == 1, totals[0].seconds == 14520, totals[0].clipped == 540, totals[0].sessions == 3 else { throw CheckFailure.pairing }
        for length in 0..<bytes.count {
            let decoded = InFocus.decode(bytes.prefix(length))
            guard decoded.allSatisfy({ events.contains($0) }) else { throw CheckFailure.truncation }
        }
        let malformed = try Data(contentsOf: URL(fileURLWithPath: root + "/malformed.segb"))
        guard InFocus.decode(malformed).isEmpty else { throw CheckFailure.malformed }
        let midnight = Date(timeIntervalSince1970: 1792195140)
        let split = FocusUsage.aggregate([FocusEvent(at: midnight.timeIntervalSince1970, starting: true, bundle: "test.app"), FocusEvent(at: midnight.timeIntervalSince1970 + 120, starting: false, bundle: "test.app")], mac: true, timeZone: TimeZone(secondsFromGMT: 0)!)
        guard split.count == 2, split.allSatisfy({ $0.seconds == 60 }) else { throw CheckFailure.midnight }
        print("Usage decoder and session fixtures passed")
    }
    private enum CheckFailure: Error { case records, pairing, truncation, malformed, midnight }
}
