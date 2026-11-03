import Foundation
import Decoders
struct Body: Encodable { let text: String; let rich: Bool; let attachments: [String]; let blocks: [NotesBlock]; let html: String; let lossless: Bool }
let body = try NotesProto.decode(Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
print(String(decoding: try encoder.encode(Body(text: body.text, rich: body.rich, attachments: body.attachments, blocks: body.blocks, html: body.html, lossless: body.lossless)), as: UTF8.self))
