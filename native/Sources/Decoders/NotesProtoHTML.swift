import Foundation

extension NotesProto {
    static func escape(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;")
    }

    static func html(_ blocks: [NotesBlock]) -> String {
        blocks.map { block in
            var text = block.runs.map { run in
                var value = escape(run.text).replacingOccurrences(of: "\u{2028}", with: "<br>")
                if run.bold { value = "<b>" + value + "</b>" }
                if run.italic { value = "<i>" + value + "</i>" }
                if run.underline { value = "<u>" + value + "</u>" }
                if run.strikethrough { value = "<s>" + value + "</s>" }
                if run.superscript == 1 { value = "<sup>" + value + "</sup>" }
                if run.superscript == -1 { value = "<sub>" + value + "</sub>" }
                var styles: [String] = []
                if let color = run.color { styles.append("color:" + color) }
                if let size = run.font_size { styles.append("font-size:\(size)pt") }
                if let font = run.font, font.count <= 128, font.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " -_.")).contains($0) }) { styles.append("font-family:'" + font + "'") }
                if let highlight = run.highlight {
                    let colors = ["purple": "#d9b8ff", "pink": "#ffb8d6", "orange": "#ffcf99", "mint": "#b0e8cc", "blue": "#b8daff"]
                    if let color = colors[highlight] { styles.append("background-color:" + color) }
                }
                if !styles.isEmpty { value = "<span style=\"" + escape(styles.joined(separator: ";")) + "\">" + value + "</span>" }
                if let link = run.link, let scheme = URL(string: link)?.scheme?.lowercased(), ["http", "https", "mailto", "notes"].contains(scheme) { value = "<a href=\"" + escape(link) + "\">" + value + "</a>" }
                return value
            }.joined()
            if text.isEmpty { text = "<br>" }
            let alignment = block.alignment.map { " style=\"text-align:" + escape($0) + "\"" } ?? ""
            let result: String
            switch block.type {
            case "title": result = "<h1" + alignment + ">" + text + "</h1>"
            case "heading":
                let tag = block.level == 2 ? "h3" : "h2"
                result = "<" + tag + alignment + ">" + text + "</" + tag + ">"
            case "monostyled": result = "<pre" + alignment + ">" + text + "</pre>"
            case "quote": result = "<blockquote" + alignment + ">" + text + "</blockquote>"
            case "list_item":
                let tag = block.list_style == "numbered" ? "ol" : "ul"
                result = "<" + tag + "><li" + alignment + ">" + text + "</li></" + tag + ">"
            case "table":
                result = "<table>" + (block.rows ?? []).map { row in "<tr>" + row.map { "<td>" + html($0.blocks) + "</td>" }.joined() + "</tr>" }.joined() + "</table>"
            case "attachment", "checklist_item", "unknown": result = "<div data-notes-read-only=\"true\">" + text + "</div>"
            default: result = "<div" + alignment + ">" + text + "</div>"
            }
            return result + html(block.children)
        }.joined()
    }
}
