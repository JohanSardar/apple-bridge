import Foundation
import Decoders

let root = CommandLine.arguments[1]
let encoder = JSONEncoder()
encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
for name in try FileManager.default.contentsOfDirectory(atPath: root).sorted() where name.hasSuffix(".typedstream") {
    let data = try Data(contentsOf: URL(fileURLWithPath: root + "/" + name))
    let body = TypedStream.decode(data)
    let object = try JSONSerialization.jsonObject(with: encoder.encode(body))
    let line = try JSONSerialization.data(withJSONObject: ["fixture": name, "body": object], options: [.sortedKeys, .withoutEscapingSlashes])
    print(String(decoding: line, as: UTF8.self))
}
let notes = CommandLine.arguments[2]
for name in try FileManager.default.contentsOfDirectory(atPath: notes).sorted() where name.hasSuffix(".gz") {
    let data = try Data(contentsOf: URL(fileURLWithPath: notes + "/" + name))
    var value: [String: Any] = ["fixture": name, "decoder": "notes"]
    if let body = try? NotesProto.decode(data, tables: ["fixture-table": Data(contentsOf: URL(fileURLWithPath: notes + "/full-table.gz"))]) {
        value["body"] = ["text": body.text, "rich": body.rich, "attachments": body.attachments, "blocks": try JSONSerialization.jsonObject(with: encoder.encode(body.blocks)), "html": body.html, "lossless": body.lossless]
    } else { value["error"] = true }
    print(String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes]), as: UTF8.self))
}
for name in try FileManager.default.contentsOfDirectory(atPath: root).sorted() where name.hasSuffix(".plist") {
    let data = try Data(contentsOf: URL(fileURLWithPath: root + "/" + name))
    let summary = try JSONSerialization.jsonObject(with: encoder.encode(MessageSummary.decode(data)))
    print(String(decoding: try JSONSerialization.data(withJSONObject: ["fixture": name, "decoder": "summary", "body": summary], options: [.sortedKeys, .withoutEscapingSlashes]), as: UTF8.self))
}
struct FocusVector: Encodable { let fixture: String; let decoder = "infocus"; let body: [FocusEvent] }
struct FocusEvent: Encodable { let time: Double; let starting: Bool; let bundle: String }
let usage = CommandLine.arguments[3]
for name in try FileManager.default.contentsOfDirectory(atPath: usage).sorted() where name.hasSuffix(".segb") {
    let events = InFocus.decode(try Data(contentsOf: URL(fileURLWithPath: usage + "/" + name))).map { FocusEvent(time: $0.time, starting: $0.starting, bundle: $0.bundle) }
    print(String(decoding: try encoder.encode(FocusVector(fixture: name, body: events)), as: UTF8.self))
}
for name in try FileManager.default.contentsOfDirectory(atPath: root).sorted() where name.hasSuffix(".plist") {
    let data = try Data(contentsOf: URL(fileURLWithPath: root + "/" + name))
    let attachments = ["synthetic-image", "synthetic-icon"]
    let result: [String: Any] = ["fixture": name, "decoder": "rich", "body": ["balloon": MessageRich.balloon(bundle: "URLBalloonProvider", payload: data, attachmentGUIDs: attachments) as Any, "app": MessageRich.balloon(bundle: "MSMessageExtensionBalloonPlugin:synthetic", payload: data, attachmentGUIDs: attachments) as Any, "sticker": MessageRich.sticker(data, target: "p:2/synthetic-parent") ?? NSNull() as Any, "chat": MessageRich.chatProperties(data)]]
    print(String(decoding: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .withoutEscapingSlashes]), as: UTF8.self))
}
