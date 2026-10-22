import Foundation

extension NotesProto {
    static func inline(_ attribute: NotesProtoMessage, text: String) throws -> NotesRun {
        var run = NotesRun(text: text)
        let weight = attribute.integer(5) ?? 0
        guard weight <= 3 else { throw NotesProtoFailure.unsupported }
        run.bold = weight == 1 || weight == 3
        run.italic = weight == 2 || weight == 3
        guard (attribute.integer(6) ?? 0) <= 1, (attribute.integer(7) ?? 0) <= 1 else { throw NotesProtoFailure.unsupported }
        run.underline = attribute.integer(6) == 1
        run.strikethrough = attribute.integer(7) == 1
        run.link = try attribute.string(9)
        if let font = try attribute.message(3) {
            try font.validate([1: 2, 2: 5, 3: 0])
            if font.fields.contains(where: { ![1, 2].contains($0.number) }) { run.unsupported = "unknown_font_attribute" }
            run.font = try font.string(1)
            if let size = font.fields.first(where: { $0.number == 2 && $0.wire == 5 }) {
                let bits = size.bytes.enumerated().reduce(UInt32(0)) { $0 | UInt32($1.element) << (8 * $1.offset) }
                let value = Double(Float(bitPattern: bits))
                guard value.isFinite, value > 0, value <= 512 else { throw NotesProtoFailure.unsupported }
                run.font_size = value
            }
        }
        if let superscript = attribute.integer(8), superscript != 0 {
            guard superscript == 1 || superscript == UInt64.max else { throw NotesProtoFailure.unsupported }
            run.superscript = superscript == 1 ? 1 : -1
        }
        if let highlight = attribute.integer(14), highlight != 0 {
            guard highlight <= 5 else { throw NotesProtoFailure.unsupported }
            run.highlight = ["purple", "pink", "orange", "mint", "blue"][Int(highlight) - 1]
        }
        if let color = try attribute.message(10) {
            try color.validate([1: 5, 2: 5, 3: 5, 4: 5])
            var components: [UInt8] = []
            for number in 1...4 {
                guard let field = color.fields.first(where: { $0.number == number && $0.wire == 5 }) else { throw NotesProtoFailure.malformed }
                let bits = field.bytes.enumerated().reduce(UInt32(0)) { $0 | UInt32($1.element) << (8 * $1.offset) }
                let value = Float(bitPattern: bits)
                guard value.isFinite, value >= 0, value <= 1 else { throw NotesProtoFailure.malformed }
                components.append(UInt8((value * 255).rounded()))
            }
            run.color = "#" + components.map { String(format: "%02x", $0) }.joined()
        }
        if let attachment = try attribute.message(12) {
            try attachment.validate([1: 2, 2: 2])
            run.attachment_id = try attachment.string(1)
            run.attachment_uti = try attachment.string(2)
        }
        return run
    }

    static func paragraphBlock(_ paragraph: NotesProtoMessage) throws -> NotesBlock {
        try paragraph.validate([1: 0, 2: 0, 4: 0, 5: 2, 8: 0])
        let style = Int(Int32(truncatingIfNeeded: paragraph.integer(1) ?? UInt64.max))
        var block = NotesBlock(type: "paragraph", text: "", runs: [])
        switch style {
        case -1: break
        case 0: block.type = "title"
        case 1, 2: block.type = "heading"; block.level = style
        case 4: block.type = "monostyled"
        case 100, 101, 102:
            block.type = "list_item"
            block.list_style = ["bullet", "dash", "numbered"][style - 100]
        case 103:
            block.type = "checklist_item"
            if let checklist = try paragraph.message(5), let checked = checklist.integer(2), checked <= 1 {
                try checklist.validate([1: 2, 2: 0])
                block.checked = checked == 1
                if let identifier = checklist.bytes(1), identifier.count == 16 {
                    let bytes = [UInt8](identifier)
                    block.checklist_id = bytes.enumerated().map { index, byte in ([4, 6, 8, 10].contains(index) ? "-" : "") + String(format: "%02x", byte) }.joined()
                } else { block.unsupported = "checklist_identifier_missing" }
            } else { block.unsupported = "checklist_state_missing" }
        default: block.type = "unknown"; block.unsupported = "unknown_paragraph_style"
        }
        if let indent = paragraph.integer(4) {
            guard indent <= 128 else { throw NotesProtoFailure.unsupported }
            if block.type != "heading" { block.level = Int(indent) }
        }
        guard (paragraph.integer(8) ?? 0) <= 1 else { throw NotesProtoFailure.unsupported }
        if paragraph.integer(8) == 1 {
            block.blockquote = true
            if block.type == "paragraph" { block.type = "quote" }
        }
        if let alignment = paragraph.integer(2) {
            guard alignment <= 3 else { throw NotesProtoFailure.unsupported }
            block.alignment = ["left", "center", "right", "justify"][Int(alignment)]
        }
        if paragraph.fields.contains(where: { ![1, 2, 4, 5, 8].contains($0.number) }) { block.unsupported = "unknown_paragraph_attribute" }
        return block
    }
}
