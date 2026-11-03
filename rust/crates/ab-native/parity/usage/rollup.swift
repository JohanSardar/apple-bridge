import Foundation
import UsageSource
import Decoders
import SourceSupport

let fixture = URL(fileURLWithPath: CommandLine.arguments[1])
let input = try JSONSerialization.jsonObject(with: Data(contentsOf: fixture)) as! [[String: Any]]
let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
for vector in input {
    let events = (vector["events"] as! [[String: Any]]).map { InFocusEvent(time: $0["time"] as! Double, starting: $0["starting"] as! Bool, bundle: $0["bundle"] as! String) }
    let zone = TimeZone(identifier: vector["zone"] as! String)!
    let blocked = (vector["blocked"] as! [[String: Double]]).map { DateInterval(start: Date(timeIntervalSince1970: $0["start"]!), end: Date(timeIntervalSince1970: $0["end"]!)) }
    let records = UsageRollup.records(events, device: "synthetic", mac: vector["mac"] as! Bool, zone: zone, blocked: blocked)
    print(String(decoding: try encoder.encode(JSON.object(["name": .string(vector["name"] as! String), "records": .object(records)])), as: UTF8.self))
}
