import Foundation
let input = try JSONDecoder().decode([String].self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
let encoder = JSONEncoder()
encoder.outputFormatting = [.sortedKeys]
for hex in input {
    guard let bits = UInt64(hex, radix: 16) else { exit(2) }
    let value = Double(bitPattern: bits)
    let encoded = try encoder.encode(value)
    print(hex + "\t" + String(decoding: encoded, as: UTF8.self))
}
if CommandLine.arguments.contains("--strings") {
    let strings = ["/", "</div>", "&<>\"'", "\u{2028}\u{2029}", "a\nb\rc\td", "\u{0000}\u{0001}\u{0008}\u{000b}\u{000c}\u{001f}", "🧪 שלום e\u{301}", "a\\/b"]
    print("@strings\t" + (try encoder.encode(strings)).base64EncodedString())
}

if CommandLine.arguments.contains("--strings") {
    struct Description: Encodable { let bits: String; let description: String }
    let values: [Double] = [14, 14.25, Double(Float(14.1)), Double(Float(0.00001)), -0.0, 1e15, 9007199254740992, 9007199254740994]
    print("@descriptions\t" + (try encoder.encode(values.map { Description(bits: String($0.bitPattern, radix: 16), description: String($0)) })).base64EncodedString())
}
