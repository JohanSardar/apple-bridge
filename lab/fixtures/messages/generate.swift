import Foundation
let value = NSMutableAttributedString(string: "[ab-test decoder] Hello 🧪 שלום e\u{301}\u{fffc} tail")
value.addAttribute(NSAttributedString.Key("__kIMMessagePartAttributeName"), value: 0, range: NSRange(location: 0, length: 34))
value.addAttribute(NSAttributedString.Key("__kIMFileTransferGUIDAttributeName"), value: "synthetic-attachment", range: NSRange(location: 34, length: 1))
value.addAttribute(NSAttributedString.Key("__kIMMentionConfirmedMention"), value: "self@example.com", range: NSRange(location: 18, length: 5))
let data = NSArchiver.archivedData(withRootObject: value)
try data.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
let long = NSAttributedString(string: String(repeating: "[ab-test long] 🧪 שלום e\u{301} ", count: 4096))
try NSArchiver.archivedData(withRootObject: long).write(to: URL(fileURLWithPath: CommandLine.arguments[1]).deletingLastPathComponent().appendingPathComponent("long.typedstream"))
let summary: [String: Any] = ["ec": ["0": [["d": 123.5, "t": data]]], "rp": [1, 3]]
try PropertyListSerialization.data(fromPropertyList: summary, format: .binary, options: 0).write(to: URL(fileURLWithPath: CommandLine.arguments[1]).deletingLastPathComponent().appendingPathComponent("edited-unsent.plist"))
print("synthetic_fixtures", 3)
