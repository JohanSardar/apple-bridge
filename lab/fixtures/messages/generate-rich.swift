import Foundation
let directory = URL(fileURLWithPath: CommandLine.arguments[1])
let value = NSMutableAttributedString(string: "🧪ABCDEFGH overlapping synthetic")
value.addAttribute(NSAttributedString.Key("__kIMTextBoldAttributeName"), value: 1, range: NSRange(location: 0, length: 5))
value.addAttribute(NSAttributedString.Key("__kIMTextItalicAttributeName"), value: 1, range: NSRange(location: 0, length: 5))
value.addAttribute(NSAttributedString.Key("__kIMTextUnderlineAttributeName"), value: 1, range: NSRange(location: 2, length: 4))
value.addAttribute(NSAttributedString.Key("__kIMTextStrikethroughAttributeName"), value: 1, range: NSRange(location: 4, length: 4))
for (index, effect) in [5, 11, 9, 8, 12, 4, 6, 10].enumerated() {
    value.addAttribute(NSAttributedString.Key("__kIMTextEffectAttributeName"), value: effect, range: NSRange(location: 2 + index, length: 1))
}
try NSArchiver.archivedData(withRootObject: value).write(to: directory.appendingPathComponent("rich-formatting.typedstream"))
let glyph = NSMutableAttributedString(string: "Synthetic \u{fffc}")
glyph.addAttribute(NSAttributedString.Key("__kIMEmojiImageAttributeName"), value: 1, range: NSRange(location: 10, length: 1))
glyph.addAttribute(NSAttributedString.Key("__kIMFileTransferGUIDAttributeName"), value: "synthetic-genmoji", range: NSRange(location: 10, length: 1))
try NSArchiver.archivedData(withRootObject: glyph).write(to: directory.appendingPathComponent("rich-genmoji.typedstream"))
