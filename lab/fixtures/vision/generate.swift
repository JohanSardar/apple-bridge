import AppKit
import PDFKit

let root = URL(fileURLWithPath: CommandLine.arguments[1])
for (language, text) in [("en", "Bridge fixture"), ("fr", "Bonjour le monde"), ("de", "Guten Morgen")] {
    let image = NSImage(size: NSSize(width: 1000, height: 180))
    image.lockFocus()
    NSColor.white.setFill()
    NSRect(x: 0, y: 0, width: 1000, height: 180).fill()
    (text as NSString).draw(at: NSPoint(x: 40, y: 60), withAttributes: [.font: NSFont.systemFont(ofSize: 52), .foregroundColor: NSColor.black])
    image.unlockFocus()
    let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
    try bitmap.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent(language + ".png"))
}
let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 800))
text.string = "Bridge fixture"
text.font = NSFont.systemFont(ofSize: 24)
try text.dataWithPDF(inside: text.bounds).write(to: root.appendingPathComponent("fixture.pdf"))
